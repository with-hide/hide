use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 2;

fn serialize_optional_shared<T: Serialize, S: serde::Serializer>(
    value: &Option<Arc<T>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value.as_deref().serialize(serializer)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CoreOptions {
    pub schema_version: u32,
    /// The account home the core reads and writes for: agent conversations,
    /// label state, saved AI settings, usage and the install kit. Absent
    /// means the process `HOME`; an embedder running with a private home
    /// (a test daemon) names it so nothing reaches the operator's.
    #[serde(default)]
    pub home: Option<String>,
    /// The machine this core runs on (`node::NodeId`), read by the host
    /// before the core is placed behind its runtime mutex. Every key naming
    /// this machine, in the snapshot and in stored state, is this id.
    pub node_id: crate::node::NodeId,
    pub herdr_socket_path: Option<String>,
    #[serde(default)]
    pub herdr_bin_path: Option<String>,
    pub app_state_path: String,
    /// Where the helper is installed on devices; `~/` is the device account's
    /// home. Absent means `remote::DEFAULT_HELPER_ROOT`.
    #[serde(default)]
    pub host_helper_root: Option<String>,
    /// The folder on devices where `hide` is linked to the copy installed
    /// with the helper; `~/` is the device account's home. Absent means
    /// `remote::DEFAULT_CLI_DIR`.
    #[serde(default)]
    pub host_cli_dir: Option<String>,
    /// Where a shell that draws separate Agent and View areas keeps each
    /// Workspace's presentation (PRD S6 D-10). Its presence is what turns
    /// those semantics on: the web daemon passes it; a client that passes
    /// nothing keeps a document in place of the terminal canvas.
    #[serde(default)]
    pub workspace_views_path: Option<String>,
    /// The state file the removed native app wrote, whose `shortcut_bindings`
    /// the core adopts once while its own set is empty, so the desktop app
    /// starts with the pane chords the operator chose there. The file is
    /// never written again; the core is the owner that outlived it.
    #[serde(default)]
    pub shortcut_import_path: Option<String>,
    /// The file that keeps every project's Local issues (`local_issues.rs`).
    /// Absent keeps them in memory for the session only, as a test core does.
    #[serde(default)]
    pub local_issues_path: Option<String>,
    /// The core starts on a move's copy whose link has not committed it:
    /// it acts on nothing outside the machine until the embedder calls
    /// `Core::release_effects` (`effects.rs`).
    #[serde(default)]
    pub effects_held: bool,
    /// What the machine this core runs on calls itself, which a window on
    /// another machine shows for the core's row (PRD core-host-node-move
    /// B2); absent where the system names none.
    #[serde(default)]
    pub machine_name: Option<String>,
    /// The embedder runs on the state folder this account's hide picks by
    /// default; only such a core writes this machine's guardrails policy,
    /// which every hook of the account reads (PRD agent-guardrails D-11). A
    /// test daemon on a relocated folder writes nothing under `~/.hide`.
    #[serde(default)]
    pub account_state_folder: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub usage: crate::usage::model::UsageSnapshot,
    pub schema_version: u32,
    pub delivery_watches: Vec<crate::delivery::watch::View>,
    pub navigator: NavigatorSnapshot,
    pub overlay: OverlaySnapshot,
    pub tab: TabSnapshot,
    pub connection: ConnectionSnapshot,
    pub zoomed: Option<String>,
    pub focused: FocusedSnapshot,
    /// Every tab's layout in the local Herdr session, keyed by the tab id
    /// each one carries. The shell draws the entry whose tab is active, so a
    /// tab switch is a lookup rather than a wait: nothing here is emptied to
    /// mark a switch in progress, and the geometry drawn is always one Herdr
    /// has already applied.
    pub pane_layouts: Vec<PaneLayoutSnapshot>,
    pub terminal: TerminalSnapshot,
    pub editor: EditorSnapshot,
    pub sessions: SessionsSnapshot,
    /// Edited rather than compared: the section carries every diff on
    /// screen, and a snapshot read tells it changed by its edit number.
    pub changes: Edited<ChangesSnapshot>,
    pub card: CheckoutCardSnapshot,
    pub git_worktrees: Option<ProjectWorktreesSnapshot>,
    pub git_worktrees_loading: bool,
    pub git_worktrees_remote: bool,
    pub worktree_removal: Option<WorktreeRemovalSnapshot>,
    pub task_operation: Option<TaskOperationSnapshot>,
    pub repository_clone: Option<RepositoryCloneSnapshot>,
    pub explorer_operation: Option<ExplorerOperationSnapshot>,
    pub issue_work: IssueWorkSnapshot,
    pub pr_work: PrWorkSnapshot,
    pub find: PaneFindSnapshot,
    pub ui_state: UiStateSnapshot,
    pub ime: ImeSnapshot,
    pub status: StatusSnapshot,
    pub pet: PetSnapshot,
    pub recent_closed: RecentClosedSnapshot,
    /// The front Workspace's presentation, present only in a shell that draws
    /// separate Agent and View areas (`CoreOptions::workspace_views_path`).
    /// Omitted rather than null otherwise, so an older client's snapshot
    /// keeps exactly its keys.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_view: Option<WorkspaceViewSnapshot>,
    /// Browser displays across retained Workspaces. The native host uses this
    /// inventory to close a hidden page when a pane closes its View remotely.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub browser_views: Vec<BrowserViewInventoryRow>,
    /// Positive browser control authority, including an area's empty state.
    /// Losing a scope permanently revokes its desktop capabilities.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub browser_scopes: Vec<BrowserAreaScopeRow>,
    #[serde(skip)]
    pub browser_views_revision: Option<u64>,
    /// Project Memory (PRD project-memory-v2): the Project a shell opened
    /// Memory for, and the cards each live agent session on this Mac was
    /// given. Its own revisioned section, absent until there is either.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemorySnapshot>,
    /// Each project's link counts and session chips (PRD link-graph D-45),
    /// absent until the link worker first answers. Its own revisioned
    /// section, off `rest`, so an agent row's change never resends it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_summaries: Option<crate::links::LinkSummariesSnapshot>,
    /// The record for the pull request or issue a panel shows, absent until
    /// a shell opens one; its own revisioned section.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_panel: Option<crate::links::LinkPanelSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "serialize_optional_shared")]
    pub session_record: Option<Arc<crate::links::ledger::SessionRecordSnapshot>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "serialize_optional_shared")]
    pub session_work_search: Option<Arc<crate::links::ledger::SessionSearchSnapshot>>,
    /// The Factory screens' summary and their actions' answers (PRD
    /// software-factory-ui), absent until the Factory host first hands one
    /// over. Its own revisioned section, stamped by edit number, so the
    /// summary is never compared under the lock.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factory: Option<Edited<crate::factory::screen::FactorySection>>,
    /// The Task page a screen opened; its own revisioned section.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factory_task: Option<Edited<crate::factory::screen::FactoryTaskSection>>,
}

/// A snapshot value that takes a new edit number whenever it may change, so
/// a snapshot read tells a changed value from an unchanged one by comparing
/// two numbers instead of the contents: the Changes section carries every
/// diff on screen, and a View document a whole file, and a read runs on
/// every terminal output burst.
///
/// The value is reachable mutably only through [`Edited::edit`] and
/// [`Edited::set`], which take the number, so no change can skip it; a
/// mutable borrow that changes nothing costs one extra send, never a missed
/// one. Numbers come from one process-wide counter, so a value moved or
/// replaced never repeats a number a stamp still holds, and two equal
/// numbers always mean equal values.
#[derive(Clone, Debug)]
pub struct Edited<T> {
    value: T,
    edit: u64,
}

fn next_edit() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl<T> Edited<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            edit: next_edit(),
        }
    }

    /// The value to change in place, under a new edit number.
    pub fn edit(&mut self) -> &mut T {
        self.edit = next_edit();
        &mut self.value
    }

    pub fn set(&mut self, value: T) {
        *self = Self::new(value);
    }

    pub fn edit_number(&self) -> u64 {
        self.edit
    }
}

impl<T> std::ops::Deref for Edited<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T: Default> Default for Edited<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

/// Equality is the value's; the edit numbers only say whether it moved.
impl<T: PartialEq> PartialEq for Edited<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T: PartialEq> PartialEq<T> for Edited<T> {
    fn eq(&self, other: &T) -> bool {
        self.value == *other
    }
}

/// On the wire an edited value is the value alone.
impl<T: Serialize> Serialize for Edited<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.value.serialize(serializer)
    }
}

/// What the front Workspace shows: its right lane (the tool, the pin, the
/// file tree and both widths) and its View area tree (PRD right-lane D-11).
/// The editor's `active_tab_id` is the document of the active area's active
/// display.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WorkspaceViewSnapshot {
    pub device_id: String,
    pub path: String,
    /// The tool the lane shows, or null while it is closed. An unpinned
    /// lane is drawn over the agents only once a call opened it (D-02).
    pub tool: Option<crate::workspace_views::Tool>,
    /// The tool the lane last showed, null for a lane that never showed
    /// one; the tool the next open returns to.
    pub last_tool: Option<crate::workspace_views::Tool>,
    /// The PRs tool's scope chip, kept for this Workspace.
    pub prs: crate::workspace_views::PrsView,
    /// Whether the lane is pinned beside Agent Views (D-03).
    pub pinned: bool,
    /// Whether the files tool shows the file tree column (D-05).
    pub tree: bool,
    /// The files tool's document width in CSS pixels, or null for the
    /// shell's default until it is resized.
    pub lane_width: Option<u32>,
    /// The other tools' and the 보기 slot's width, the same way.
    pub panel_width: Option<u32>,
    /// The tree column's width, the same way.
    pub tree_width: Option<u32>,
    /// The last width request the core accepted for this Workspace. Runtime
    /// only, so a shell can distinguish its latest intent from an older echo.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_request_id: Option<String>,
    /// The number of this Workspace's last lane call, 0 for none since the
    /// core started; a number past `lane_calls` as the shell last read it is
    /// a call the shell shows by drawing the lane (D-02, B16).
    pub lane_called: u64,
    /// How many lane calls the core has numbered, in any Workspace.
    pub lane_calls: u64,
    /// Whether this is the Workspace the operator last chose, now or before a
    /// restart, which the shell opens on; any other front starts on Main
    /// (D-11).
    pub resumed: bool,
    pub layout: ViewLayoutSnapshot,
    /// The Agent areas, for a Workspace whose tabs the core arranges (this
    /// machine's and a node's that dialed in); `None` on a device this core
    /// dials, whose Herdr shows one tab.
    pub agent_layout: Option<AgentLayoutSnapshot>,
    /// The operator's comments on this checkout's Views (PRD
    /// view-opener-feedback); absent while there are none and no Send to say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feedback: Option<ViewFeedbackSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BrowserViewInventoryRow {
    pub device_id: String,
    pub path: String,
    pub area_id: String,
    pub view_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BrowserAreaScopeRow {
    pub device_id: String,
    pub path: String,
    pub area_id: String,
    /// Stable while authority remains present; a regrant gets a new value
    /// even when the shell never receives the intervening revocation.
    pub incarnation: u64,
}

/// Agent tab identities only; pane contents remain in checkout.tabs.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AgentLayoutSnapshot {
    pub waiting: usize,
    pub root: crate::split_tree::Node<crate::agent_layout::Tab>,
    pub active_area: String,
    pub canvases: BTreeMap<String, String>,
    pub limits: ViewLimitsSnapshot,
    pub display_count: usize,
}

/// The front Workspace's View areas (PRD S7): the tree the shell draws, the
/// area in use, and the caps a split or an open is refused at.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ViewLayoutSnapshot {
    pub root: ViewNodeSnapshot,
    pub active_area: String,
    pub limits: ViewLimitsSnapshot,
    pub display_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ViewLimitsSnapshot {
    pub areas: usize,
    pub depth: usize,
    pub displays: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewNodeSnapshot {
    Area(ViewAreaSnapshot),
    Split(ViewSplitSnapshot),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ViewAreaSnapshot {
    pub id: String,
    pub active: Option<String>,
    pub displays: Vec<ViewDisplaySnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ViewSplitSnapshot {
    pub id: String,
    pub axis: crate::view_layout::SplitAxis,
    pub ratio: f32,
    pub first: Box<ViewNodeSnapshot>,
    pub second: Box<ViewNodeSnapshot>,
}

/// One display: which document it shows and whether it can show it now.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ViewDisplaySnapshot {
    pub id: String,
    /// The editor tab (document buffer) it shows: set while `open`, and for
    /// an `unavailable` display whose unavailable tab exists.
    pub tab_id: Option<String>,
    pub path: String,
    pub label: String,
    pub kind: crate::view_layout::DisplayKind,
    pub committed: Option<bool>,
    pub preview: bool,
    pub state: ViewDisplayState,
    /// Why a `waiting` or `unavailable` display shows no document.
    pub reason: Option<String>,
    /// A browser display's address and its page's title (issue 155); absent
    /// for a file or a diff, and the title until the page reports one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// A browser display's load stamp: a larger one than the host last
    /// loaded asks it to load `url` again. Zero for a file or a diff.
    #[serde(skip_serializing_if = "load_unset")]
    pub load: u64,
    /// The agent that last opened it (PRD view-opener-feedback D-03);
    /// absent for a View the operator opened.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opener: Option<ViewOpenerSnapshot>,
    /// The operator's comments on it that are not sent, and of those the
    /// ones whose letter expired (B14, B29).
    #[serde(skip_serializing_if = "count_unset")]
    pub comments: usize,
    #[serde(skip_serializing_if = "count_unset")]
    pub not_delivered: usize,
}

fn load_unset(load: &u64) -> bool {
    *load == 0
}

fn count_unset(count: &usize) -> bool {
    *count == 0
}

/// A View's opener as the screen shows it: its name and logo, and the pane
/// it is in now, found by its terminal and session, never by a pane id
/// (D-23, B40).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ViewOpenerSnapshot {
    pub name: String,
    pub provider: String,
    pub device_id: String,
    /// The pane the agent is in now; absent while it is closed (B6).
    pub pane_id: Option<String>,
    /// Whether `Reopen and send` can start its session again (D-10).
    pub resumable: bool,
}

/// The operator's comments on the front Workspace's Views and where their
/// Send stands (PRD view-opener-feedback). Absent while there is nothing.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ViewFeedbackSnapshot {
    /// The open comments on the Views the layout shows, with their text.
    pub comments: Vec<ViewCommentSnapshot>,
    /// Every recipient with open comments in this checkout, those on closed
    /// Views included (B30).
    pub groups: Vec<ViewFeedbackGroupSnapshot>,
    /// The Views the layout shows that a live agent opened, one row per
    /// thing shown (B8).
    pub opened: Vec<OpenedViewSnapshot>,
    /// The last Send of this checkout, while the screen says it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send: Option<ViewFeedbackSendSnapshot>,
    /// The comment the last delete removed, which Undo brings back (B43).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ViewCommentSnapshot {
    pub id: String,
    /// The displays of the layout it shows on.
    pub display_ids: Vec<String>,
    pub kind: crate::view_layout::DisplayKind,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub committed: Option<bool>,
    pub anchor: crate::view_feedback::Anchor,
    pub note: String,
    pub shot: bool,
    pub changed: bool,
    pub not_delivered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// The group it is sent with, `""` for a View no agent opened.
    pub group: String,
    /// Its place in its group's letter, from 1 (B11).
    pub number: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ViewFeedbackGroupSnapshot {
    /// The recipient's key, `""` for comments on Views no agent opened,
    /// whose Send asks for a recipient (B18).
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<ViewOpenerSnapshot>,
    pub comments: usize,
    pub views: usize,
    pub not_delivered: usize,
    /// The displays of the layout that hold some of its comments.
    pub display_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OpenedViewSnapshot {
    pub pane_id: String,
    pub view_id: String,
    pub kind: crate::view_layout::DisplayKind,
    pub label: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub comments: usize,
}

/// Where a Send stands (B21, B27, B28, B20).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ViewFeedbackSendSnapshot {
    pub request_id: String,
    pub state: ViewFeedbackSendState,
    pub count: usize,
    /// The recipient's name.
    pub to: String,
    /// The recipient's pane, for Go to pane.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    /// The device a Send waits for, or whose cap or copy failed, by name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<ViewFeedbackSendReason>,
    pub at_unix_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewFeedbackSendState {
    Sending,
    Sent,
    Waiting,
    Reopening,
    Failed,
}

/// Why a Send did not go, each a sentence the screen says (B27, B20).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewFeedbackSendReason {
    AttachmentsFull,
    LetterUnsaved,
    LettersFull,
    ShotCopy,
    ReopenFailed,
    Closed,
}

/// `waiting` while its Workspace's root cannot be read yet (a device helper
/// or catalog not ready, or this machine's root not opened), `opening` while
/// its file is read, `open` with its document, `unavailable` when the read
/// failed (Close and Retry).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewDisplayState {
    Open,
    Opening,
    Waiting,
    Unavailable,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct RecentClosedSnapshot {
    pub count: usize,
    pub top_label: Option<String>,
    pub restoring: bool,
    pub notices: Vec<RecentClosedNoticeSnapshot>,
    /// Close reservations whose topology is not confirmed yet. These are
    /// deliberately separate from `count`: a pending close must not evict a
    /// confirmed undo entry or become reopenable by accident.
    pub pending: Vec<RecentClosedPendingSnapshot>,
    pub can_reopen: bool,
    pub reopen_blocked_reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RecentClosedNoticeSnapshot {
    pub pane_id: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RecentClosedPendingSnapshot {
    pub key: String,
    pub target_id: String,
    pub label: String,
    pub phase: String,
    pub checking: bool,
    pub message: Option<String>,
    pub retryable: bool,
}

/// A core-owned asynchronous mutation. The shell renders this record in the
/// existing request-scoped affordance for the subject and never infers a
/// server confirmation from the transport response alone.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AsyncOperationSnapshot {
    pub id: String,
    pub kind: String,
    pub target_id: String,
    pub scope_id: String,
    pub phase: String,
    pub stage: String,
    pub started_at_unix_ms: u64,
    pub deadline_at_unix_ms: Option<u64>,
    pub message: Option<String>,
    pub retryable: bool,
}

/// What a pane search found, over the pane's whole scrollback.
///
/// The count is the reason this crosses the wire at all: the terminal view the
/// shell draws holds only the visible rows, so a counter it computed itself
/// would report what is on screen and call it the total.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PaneFindSnapshot {
    /// Which pane the result belongs to, so a result that lands after the
    /// operator has moved on is ignored rather than shown over another pane.
    pub pane_id: Option<String>,
    pub term: String,
    /// 1-based position of the current match, or 0 when there is none.
    pub index: usize,
    pub total: usize,
    /// Herdr capped the history it returned, so `total` counts what was
    /// searched rather than everything the pane has ever printed.
    pub truncated: bool,
    pub unavailable_reason: Option<String>,
    /// Where the last `pane_find_open` sent the search, named by the request
    /// that asked, so the shell acts on its own answer once and never on a
    /// late one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opened: Option<PaneFindOpened>,
}

/// The answer to one `pane_find_open`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PaneFindOpened {
    pub request_id: String,
    pub route: PaneFindRoute,
}

/// Where ⌘F's search went for an agent with its own find.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneFindRoute {
    /// The agent's own search is open in the pane and takes the typing.
    Agent,
    /// Herdr holds the pane's history, or the agent's search could not be
    /// opened (`unavailable_reason` says why), so Hide's find bar opens.
    Bar,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PetSnapshot {
    pub visible: bool,
    /// `connected` while Herdr session sync is healthy; otherwise the sync
    /// failure state, so a missing socket is never a silent idle.
    pub connection: String,
    pub connection_message: Option<String>,
    pub pose: String,
    pub sleep_phase: String,
    pub roam_allowed: bool,
    pub badges: PetBadgesSnapshot,
    /// Unseen panes in click order: oldest observation first, snapshot order
    /// as the tie-break.
    pub attention_pane_ids: Vec<String>,
    pub origin: Option<PetOriginSnapshot>,
    pub shortcut: Option<String>,
    pub shortcut_error: Option<String>,
    pub theme_id: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PetBadgesSnapshot {
    /// The four groups the sidebar draws, counted once. `needs_you` is the
    /// pet's "act now" number and `done` is what finished unseen, so the badge
    /// row and the sidebar cannot disagree.
    pub needs_you: usize,
    pub done: usize,
    pub working: usize,
    pub seen: usize,
    /// Retained agents on a server that stopped answering. They are counted
    /// separately because a stale count of what is waiting would be a lie.
    pub disconnected: usize,
    /// In-process subagents Hide's hook reported as working, summed over the
    /// agents on an answering server. It is the one count the hook can vouch
    /// for; an uninstrumented session contributes nothing, not a zero.
    pub subagents_active: u32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PetOriginSnapshot {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NavigatorSnapshot {
    pub agent_scope: crate::agent_state::Scope,
    pub root_path: Option<String>,
    /// The focused local checkout's History scope. A registered subfolder may
    /// be narrower than the Git checkout used by Explorer and editor tabs.
    pub changes_root_path: Option<String>,
    pub focused_device_id: Option<String>,
    pub focused_workspace_id: Option<String>,
    pub focused_checkout_id: Option<String>,
    pub devices: Vec<DeviceSnapshot>,
    pub workspaces: Vec<WorkspaceSnapshot>,
    /// Project rows the core grouped at the bottom of each device's Projects
    /// list. `workspaces` remains the one authoritative row collection so
    /// search, focus, and project navigation never lose a folded project.
    pub inactive_projects: Vec<InactiveProjectGroupSnapshot>,
    /// Each device's projects with no agent row and no sleeping
    /// conversation, folded after its active projects and before its
    /// Inactive fold (PRD sidebar-titlebar-refresh D-01). Pinned projects,
    /// the project in front and Inactive projects are never here; the open
    /// state is the `no_agents/<device>` key of `session_open_folds`.
    pub no_agent_projects: Vec<InactiveProjectGroupSnapshot>,
    pub agents: Vec<SidebarAgentSnapshot>,
    /// The checkouts Hide AI started an agent in, by device and path. A list
    /// beside the checkouts, not a field on them, because a checkout row is
    /// rebuilt from Git and Herdr at several places and would lose a value
    /// laid on it; the shell joins it by `(device_id, path)`.
    pub hide_ai_checkouts: Vec<HideAiCheckoutSnapshot>,
    /// Durable absent-pane sessions are not live agents or pane addresses.
    pub sleeping_sessions: Vec<crate::agent_sleep::SleepingSessionSnapshot>,
    /// The archived sessions, newest archived first (PRD session-archive
    /// D-09); a row whose root is live again only feeds that root's record.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archived_sessions: Vec<crate::session_archive::ArchivedSessionSnapshot>,
    /// The panes a Factory runs its workers in, sorted: their lifecycle is
    /// the Factory's, so the shell offers no `세션 보관` there (PRD
    /// session-archive D-25).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub factory_worker_panes: Vec<String>,
}

/// Hide AI started this agent, read from the coordination ledger's record of
/// the spawn and never from a branch name. `task` is what it was started for.
///
/// A kind with a home makes the row the home's: it leaves the operator's
/// lists and reaches Needs You only on the operator's turn (PRD
/// hide-ai-work-home D-06, D-13). A kind this build has no home for keeps
/// its mark and stays an operator row (D-19).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct HideAiSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<crate::coordination::SpawnTask>,
    /// The screen that owns this agent's work; absent for a kind this build
    /// has no home for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home: Option<crate::coordination::TaskHome>,
    /// The catalog Project whose home shows the work, once the home has
    /// recorded the attempt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// What the home waits for the operator to do, beyond what the agent
    /// itself asks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn: Option<HomeTurn>,
}

/// The home's turn: the operator is to act on the work the agent did
/// (D-14). The agent's own question, approval or block is the row's demand,
/// not this.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HomeTurn {
    /// The pull request is open, ready, passing or unchecked, and nobody
    /// asked for changes: it waits for the operator's review.
    PrReview { pr_number: u64 },
    /// Checks failed and the agent is not at work on them.
    CiFailed { pr_number: u64 },
}

/// A checkout Hide AI made for an agent it started.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HideAiCheckoutSnapshot {
    pub device_id: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<crate::coordination::SpawnTask>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct InactiveProjectGroupSnapshot {
    pub device_id: String,
    pub expanded: bool,
    /// IDs in the same recent-activity order as `NavigatorSnapshot.workspaces`.
    pub project_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeviceSnapshot {
    pub agent_scope: crate::agent_state::Scope,
    pub id: String,
    /// The name the operator registered the device under. The core's own
    /// row has none: its name is what its system calls it (This Mac, This
    /// PC, This computer), which each shell says in its own language from
    /// `host.platform`, so no English sentence travels the wire.
    pub label: Option<String>,
    pub kind: String,
    /// `local` for this Mac; for an SSH device `ready`, `unavailable` or
    /// `disabled`, read off its remote status.
    pub state: String,
    /// Why an SSH device is not `ready`, in the words its remote status
    /// carries; `None` while it is.
    pub message: Option<String>,
    /// Which trust or sign-in step refused the connection, when one did:
    /// `host_key_changed`, `host_key_unknown` or `authentication`, each
    /// needing a different action from the operator (PRD S5.5 B38); `None`
    /// for any other state.
    pub problem: Option<String>,
    pub ssh_alias: Option<String>,
    /// The Herdr socket the registration names on the device, if any.
    pub herdr_socket_path: Option<String>,
    pub agent_count: u32,
    /// The last connection test the operator asked for, or the one running.
    pub test: Option<DeviceTestSnapshot>,
    /// Where file and Git work for this device runs, and whether it may.
    pub host: DeviceHostSnapshot,
    /// What Hide's install kit has put on this machine (PRD device-parity
    /// B7): the same parts, in the same order, on This Mac and on every
    /// device.
    pub kit: KitSnapshot,
    /// On the core's row only: what its machine calls itself, which a window
    /// on another machine shows in place of "This Mac" (PRD
    /// core-host-node-move B2).
    pub machine_name: Option<String>,
    /// A node that dials this core, which the core never dials: it has no
    /// SSH alias, connection test or helper consent here (PRD
    /// core-host-node-remote-core D-04).
    pub dials_in: bool,
    /// The device's Herdr server stays on an older release than this
    /// build's pinned Herdr, as its node last found after a kit pass (PRD
    /// device-herdr-update B8); `None` while it runs the pin, a newer
    /// release, or nothing the node could read.
    pub herdr_behind: Option<HerdrBehindSnapshot>,
}

/// A device's Herdr server older than the pin, and why it stays there,
/// which decides the operator's step: `cannot_hand_off` is updated on that
/// machine, `handoff_failed` is restarted there when its work is safe.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HerdrBehindSnapshot {
    pub server_version: String,
    pub version: String,
    pub cause: hide_kit::device_herdr::BehindCause,
}

/// One machine's install kit as Settings shows it (PRD device-parity B7, B8).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct KitSnapshot {
    /// Why the kit does not run on this machine at all: a daemon outside the
    /// app, a device with no consent or an unsupported platform. The parts
    /// are then empty and no Reinstall is offered.
    pub unavailable: Option<String>,
    /// An install is running for this machine; Reinstall shows its progress.
    pub busy: bool,
    /// Every part, in the kit's own order; empty until the machine was first
    /// checked.
    pub components: Vec<KitComponentSnapshot>,
    /// Every agent Hide has an adapter for, with its switch and the state of
    /// its skill and hook on this machine (issue #517); empty until the
    /// machine was first checked, and from a helper that predates agents.
    pub agents: Vec<KitAgentSnapshot>,
    /// Whether any part or any agent that is on is outdated, missing,
    /// removed or failed, which is the only time the row offers Reinstall.
    pub offers_reinstall: bool,
    /// Another registered device that reaches the same account on the same
    /// machine, by its label; removing this device then leaves the kit there
    /// for it.
    pub shares_account_with: Option<String>,
    /// What the machine's Codex answered about its shared daemon, for the
    /// `--no-daemon` start flag (`codex_launch`): `true` when it has the
    /// setting, `false` when it is confirmed older and has none, `None`
    /// when there is no answer (no Codex, or the read failed).
    pub codex_daemon: Option<bool>,
    /// Whether that Codex starts the shared daemon on its own now (`None`
    /// with no answer or an older Codex). With it on, a Codex pane that
    /// reports nothing is read as running on the shared server (B27).
    pub codex_daemon_on: Option<bool>,
    /// Whether a shared daemon answers now (`None` with no readable answer).
    /// With the setting already off, a daemon that still answers keeps a
    /// Codex pane on the shared server (PRD codex-daemon-apply D-07, B9). The
    /// core reads it into the pane's reason; the shell never does.
    #[serde(skip)]
    pub codex_daemon_running: Option<bool>,
    /// The daemon answer the kit could not read, for the log only.
    #[serde(skip)]
    pub codex_daemon_unreadable: Option<String>,
    /// Why the kit had no answer for `codex_daemon`, for the log only.
    #[serde(skip)]
    pub codex_capability_unread: Option<hide_kit::CodexReadFailure>,
    /// How the last turn-off of the shared server ended, until the next one
    /// starts; the pane popover reads it (B30).
    pub codex_daemon_off: Option<CodexDaemonOffSnapshot>,
    /// A read asked with Check again is under way on this machine: set when
    /// the request is queued and cleared by whatever answer lands first (B10).
    pub checking: bool,
    /// Why this machine's last read failed, until a read lands; the report
    /// the machine last gave stays as it was (B10).
    pub check_failed: Option<KitCheckFailure>,
    /// The machine's last pass or read stopped before changing anything:
    /// what stopped it, for one line with Try again. The parts and agents
    /// are then empty, and the kit's own words are in the log.
    pub refused: Option<KitRefusalSnapshot>,
}

/// Why a machine's kit pass stopped, as the screen words it (`hide_kit::KitRefusal`
/// without its detail, which stays in the log).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct KitRefusalSnapshot {
    pub cause: hide_kit::RefusalCause,
    /// The folder or file at fault, as that machine spells it.
    pub path: Option<String>,
    /// The variable that chose Hide's state folder at fault, by name.
    pub variable: Option<hide_kit::layout::StateVariable>,
    /// The command that stops this machine's background service, so the
    /// next launch reads `variable` again.
    pub stop: Option<String>,
}

impl From<&hide_kit::KitRefusal> for KitRefusalSnapshot {
    fn from(refusal: &hide_kit::KitRefusal) -> Self {
        Self {
            cause: refusal.cause,
            path: refusal.path.clone(),
            variable: refusal.variable,
            stop: refusal.stop.clone(),
        }
    }
}

/// Why a kit read did not come back with a report. A code, never the call's
/// own words (the words go to the diagnostic log).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KitCheckFailure {
    /// The call to the machine's helper ended without a report.
    ReadFailed,
}

/// The outcome of the operator's request to turn Codex's shared server off on
/// one machine. A failure carries a code, never the command's own words.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CodexDaemonOffSnapshot {
    Pending,
    Done,
    Failed { reason: CodexDaemonOffFailure },
}

pub use hide_kit::CodexDaemonOffFailure;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct KitComponentSnapshot {
    pub id: hide_kit::ComponentId,
    pub label: String,
    pub state: hide_kit::ComponentState,
    pub reason: Option<String>,
    pub location: Option<String>,
}

/// One agent on one machine: its switch, and what Hide put there for it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct KitAgentSnapshot {
    pub id: String,
    pub label: String,
    pub availability: hide_kit::Availability,
    pub enabled: bool,
    /// The machine's record holds the operator's own choice for this agent.
    /// An agent that is on only by default has none, so a shell keeps an
    /// agent under Installed once its program is gone only when this is true
    /// (PRD settings-cleanup B9, D-07).
    pub chosen: bool,
    pub skill: KitPieceSnapshot,
    /// `None` for an agent that gets the skill only.
    pub hook: Option<KitPieceSnapshot>,
    /// Herdr's own integration for the agent, installed through the
    /// machine's Herdr CLI. Every supported agent has one; `None` only from a
    /// device helper whose build predates the field.
    pub herdr: Option<KitPieceSnapshot>,
    /// Hide does only some of what it does for Claude Code with this agent:
    /// the row wears the Partial chip whether or not the agent is on (PRD
    /// settings-cleanup B18).
    pub partial: bool,
    /// Every feature of the Partial popover, in the table's order, with
    /// whether this build does it for the agent (`hide_kit::agents::Feature`).
    pub features: Vec<KitFeatureSnapshot>,
    /// How many of the agent's sessions run on this machine now, for the
    /// row's "N sessions" (B16); a sleeping agent is not running and is not
    /// counted. `None` for an agent whose sessions Hide does not read (B19)
    /// and for a machine whose sessions are not read yet.
    pub sessions: Option<u32>,
    /// The official page the adapter's answers come from.
    pub doc_url: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct KitFeatureSnapshot {
    pub id: hide_kit::Feature,
    pub supported: bool,
}

impl KitAgentSnapshot {
    /// Whether Reinstall would change something for this agent: only an
    /// agent that is on has pieces to repair.
    pub fn needs_attention(&self) -> bool {
        self.enabled
            && (self.skill.state.needs_attention()
                || self
                    .hook
                    .as_ref()
                    .is_some_and(|hook| hook.state.needs_attention())
                || self
                    .herdr
                    .as_ref()
                    .is_some_and(|herdr| herdr.state.needs_attention()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct KitPieceSnapshot {
    pub state: hide_kit::ComponentState,
    pub reason: Option<String>,
    pub location: Option<String>,
}

impl From<&hide_kit::PieceReport> for KitPieceSnapshot {
    fn from(piece: &hide_kit::PieceReport) -> Self {
        Self {
            state: piece.state,
            reason: piece.reason.clone(),
            location: piece.location.clone(),
        }
    }
}

impl KitSnapshot {
    pub fn from_report(report: &hide_kit::KitReport) -> Self {
        let components = report
            .components
            .iter()
            .map(|part| KitComponentSnapshot {
                id: part.id,
                label: part.id.label().to_owned(),
                state: part.state,
                reason: part.reason.clone(),
                location: part.location.clone(),
            })
            .collect::<Vec<_>>();
        let agents = report
            .agents
            .iter()
            .filter_map(|agent| {
                let row = hide_agent_adapter::adapter(&agent.id)?;
                Some(KitAgentSnapshot {
                    id: row.id.to_owned(),
                    label: agent.label.clone(),
                    availability: agent.availability,
                    enabled: agent.enabled,
                    chosen: agent.chosen,
                    skill: (&agent.skill).into(),
                    hook: agent.hook.as_ref().map(Into::into),
                    herdr: agent.herdr.as_ref().map(Into::into),
                    partial: row.basic(),
                    features: hide_agent_adapter::Feature::ALL
                        .into_iter()
                        .map(|id| KitFeatureSnapshot {
                            id,
                            supported: row.supports(id),
                        })
                        .collect(),
                    sessions: None,
                    doc_url: agent.doc_url.clone(),
                })
            })
            .collect::<Vec<_>>();
        Self {
            unavailable: None,
            busy: false,
            // Reinstall repairs what is on: a hook part of an agent that is
            // switched off is not a repair, and the pass would refuse it.
            offers_reinstall: components.iter().any(|part| {
                part.state.needs_attention()
                    && hide_kit::agents::agent_of_part(part.id).is_none_or(|agent| {
                        report
                            .agents
                            .iter()
                            .find(|row| row.id == agent)
                            .is_none_or(|row| row.enabled)
                    })
            }) || report
                .agents
                .iter()
                .any(hide_kit::AgentReport::needs_attention),
            components,
            agents,
            shares_account_with: None,
            codex_daemon: report.codex_daemon,
            codex_daemon_on: report.codex_daemon_on,
            codex_daemon_running: report.codex_daemon_running,
            codex_daemon_unreadable: report.codex_daemon_unreadable.clone(),
            codex_capability_unread: report.codex_capability_unread,
            codex_daemon_off: report
                .codex_daemon_off
                .as_ref()
                .map(|outcome| match outcome {
                    hide_kit::CodexDaemonOff::Done { .. } => CodexDaemonOffSnapshot::Done,
                    hide_kit::CodexDaemonOff::Failed { reason, .. } => {
                        CodexDaemonOffSnapshot::Failed { reason: *reason }
                    }
                }),
            checking: false,
            check_failed: None,
            refused: report.refused.as_deref().map(Into::into),
        }
    }

    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            unavailable: Some(reason.into()),
            ..Self::default()
        }
    }

    /// Whether a Codex started by hand on this machine joins the shared
    /// server: the setting starts it, or a daemon still answers after the
    /// setting went off (PRD settings-cleanup B27; codex-daemon-apply D-07,
    /// B9). After a stop that did not take effect, only a read that says no
    /// daemon answers ends it, so an answer Hide could not read keeps the
    /// retry on offer (B7). The pane popover's turn-off is offered exactly
    /// while this holds.
    pub fn shares_codex_server(&self) -> bool {
        let stop_failed = matches!(
            self.codex_daemon_off,
            Some(CodexDaemonOffSnapshot::Failed {
                reason: CodexDaemonOffFailure::StopFailed
            })
        );
        self.codex_daemon_on == Some(true)
            || self.codex_daemon_running == Some(true)
            || stop_failed && self.codex_daemon_running.is_none()
    }
}

/// A device's file host as the operator reads it in Settings and wherever a
/// file or Git action needs it (PRD S5.5 B36, B50-B52).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct DeviceHostSnapshot {
    /// `this_machine` for the daemon's own machine; for a device `none`,
    /// `granted`, or `outdated` when the consent covers an older scope.
    pub consent: String,
    /// The install root the consent names or would name. On this machine's
    /// row it is the root a new device consent would name, so the add form
    /// can say where the helper goes before the operator agrees.
    pub helper_root: Option<String>,
    /// The folder the device's `hide` command is linked in, named the same
    /// way as `helper_root`.
    pub cli_dir: Option<String>,
    pub contract: u32,
    /// `user@host:port (SHA256:...)` the consent is bound to, once bound.
    pub bound_identity: Option<String>,
    pub granted_at_unix_ms: Option<u64>,
    /// `ready`, `connecting`, `not_allowed`, `identity_changed`,
    /// `unsupported` (this build cannot serve the device) or `unavailable`.
    pub state: String,
    pub message: Option<String>,
    /// `macos aarch64` as the helper reported it.
    pub platform: Option<String>,
    pub helper_path: Option<String>,
}

/// One staged connection test of an SSH device: SSH, authentication, Herdr,
/// protocol, PTY, SFTP and Git, each with the host's answer.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeviceTestSnapshot {
    /// `running`, `passed` or `failed`.
    pub state: String,
    pub checked_at_unix_ms: Option<u64>,
    pub stages: Vec<DeviceTestStageSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeviceTestStageSnapshot {
    pub stage: String,
    /// `pending`, `passed` or `failed`.
    pub state: String,
    pub detail: String,
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

/// An agent's state on four independent lifecycle axes, plus the values the shell draws
/// from them.
///
/// The axes answer four different questions that a single flat state string
/// used to mix: what the agent needs from the operator (`demand`), whether it
/// is running (`activity`), whether it has reported a completion (`completed`),
/// and whether the operator has looked at it since it last changed (`unread`).
/// Everything below `unread` is derived here so the shell only draws (design
/// rule 4).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SidebarAgentSnapshot {
    /// Raw Herdr session bound to destructive close approval, never a label fallback.
    #[serde(skip_serializing)]
    pub close_session: Option<crate::sidebar::SessionAgentSessionPayload>,
    pub state: crate::agent_state::RowState,
    pub resolved: Option<crate::agent_state::sessions::Resolution>,
    /// Resolved within the last 24 hours, so Sessions still lists it under Resolved.
    pub resolved_recent: bool,
    pub escalation: Option<crate::agent_state::escalation::Escalation>,
    /// How many letters wait on this pane because a draft may stand in its
    /// composer, once it has rested long enough for the operator to be shown
    /// them (`doorbell::letters_waiting`): the pane header's Deliver now and
    /// the row's envelope. Absent from the wire while none waits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub letters_waiting: Option<u32>,
    /// On a lineage root: every raised descendant, lead first (PRD D-10, D-27).
    pub raised: Vec<crate::agent_state::escalation::RaisedAsk>,
    /// The one mark this row wears for its descendants while folded (PRD D-40).
    pub descendant_mark: Option<crate::agent_state::escalation::DescendantMark>,
    /// The label line of the most recently changed working descendant, the
    /// Sessions line of a root waiting on its children (PRD B31).
    #[serde(skip_serializing)]
    pub descendant_line: Option<String>,
    pub id: String,
    /// The name someone gave the agent in Herdr (`hide agent spawn --name`,
    /// `herdr agent rename`), which ⌘K also finds it by; absent when it has
    /// none or only one Hide made up from its pane (`fork::hide_made_name`).
    /// `id` falls back to the pane id and a device's row replaces it, so it
    /// cannot stand in for this.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub herdr_name: Option<String>,
    /// The name Herdr reports the agent by, a name Hide made up included,
    /// which a pane's launch record is matched by (`runtime::agent_choice`).
    /// Never on the wire.
    #[serde(skip_serializing)]
    pub started_name: Option<String>,
    /// What this pane declares about the agent it was forked from: the
    /// source's pane and the digest of the session it hosted
    /// (`wire::fork_declaration`). A fork is the operator's root, so this is
    /// provenance and never enters ownership.
    #[serde(skip_serializing)]
    pub declared_fork: Option<(String, Option<String>)>,
    /// Present on a pane Hide's Fork agent made, after its title on every
    /// row and header that draws the fork mark
    /// (`agent_state::apply_fork_sources`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<ForkSourceSnapshot>,
    pub pane_id: String,
    pub workspace_label: String,
    /// The checkout the agent's pane is in, once the navigator has placed it.
    #[serde(default)]
    pub checkout_label: Option<String>,
    pub agent_kind: String,
    /// What the agent needs from the operator: `question`, `approval`, `error`,
    /// or `none`. Herdr's `blocked` lifecycle is an approval.
    pub demand: String,
    /// Whether the agent is running: `working`, `stopped`, or `unknown`.
    /// Herdr's `done` and `idle` are the same activity. Completion and Hide's
    /// pane-level read state remain separate axes.
    pub activity: String,
    /// Whether Herdr reported a completed turn. A newly
    /// opened agent can be stopped while it waits for its first instruction;
    /// that ready state is not a completion and must not appear as Done.
    #[serde(skip_serializing)]
    pub completed: bool,
    /// Whether this pane has changed since the operator last had it focused.
    /// Owned by Hide per pane, never by Herdr's tab-scoped seen.
    pub unread: bool,
    /// An approval prompt waits on this pane right now: Herdr reports one, or
    /// the agent's session says its plan waits for the operator's approval.
    /// It holds the row in Needs You whether or not the operator has read it.
    pub blocked: bool,
    /// Derived: `needs_you`, `done`, `working`, or `seen`.
    pub group: String,
    /// Derived: whose move is next, the one mark every surface draws.
    pub mark: crate::agent_state::RowMark,
    /// Derived: rows in Needs You and Done are drawn bright, the rest subdued.
    pub emphasized: bool,
    /// Derived: the status code the screen turns into a word. No view shows an
    /// axis value.
    pub status_code: AgentStatusCode,
    /// Derived: closing this pane would interrupt work or discard a result the
    /// operator has not read.
    pub requires_close_confirmation: bool,
    /// Derived: the activity evidence is incomplete, so a destructive close
    /// must wait for a fresh status rather than assuming the pane is idle.
    pub requires_close_status_check: bool,
    /// The title every surface calls this agent by; `sidebar.rs` owns the
    /// ladder that picks it (PRD D-01).
    pub identity_label: String,
    /// The label's one-line progress sentence (`labels`).
    #[serde(skip_serializing)]
    pub progress: Option<String>,
    /// The one action the operator is being asked for, at most 40 characters.
    #[serde(skip_serializing)]
    pub expected_reply: Option<String>,
    /// Derived: the row's second line, chosen by the group from the three
    /// sentences above (PRD D-06). `None` draws no sentence.
    pub detail: Option<String>,
    /// Everything the agent last said through its hooks, uncut: the request
    /// it waits on (`expected_reply`), then what it did (`progress`), one per
    /// line. `detail` is the row's one chosen sentence cut to the row; this is
    /// the whole of both, which the Overview's node popover shows (PRD
    /// overview-lenses-tiles-agents D-50). Absent when neither was reported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Bounded native question/plan content for the current session and
    /// state. Missing source content remains absent rather than inferred.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_turn: Option<hide_session::turns::UserTurnFact>,
    /// Derived: whether the row draws its status word. It leaves working and
    /// read rows, where the mark already says it, and stays on rows that
    /// still concern the operator.
    pub status_word_visible: bool,
    /// When the core last saw this agent change state, in epoch
    /// milliseconds; `None` before the core has observed it. The shell turns
    /// it into the elapsed time on its own one-second clock, so time passing
    /// never republishes the snapshot (PRD labels-in-hided D-07).
    pub changed_at_unix_ms: Option<u64>,
    /// The ordering key: `changed_at_unix_ms` as thirteen digits when the
    /// core has it, otherwise Herdr's state change sequence zero-padded.
    pub last_activity: String,
    /// Herdr's own state change sequence, one of the three inputs to a pane's
    /// read record.
    #[serde(skip_serializing)]
    pub state_change_seq: Option<u64>,
    /// The conversation id this agent is running, kept only when Herdr recorded
    /// the session as an id. A session recorded as a path is dropped here,
    /// because neither agent's fork command takes one. The agent row's Copy
    /// session id reads it (PRD sidebar-context-menus D-06); absent from the
    /// wire when Herdr recorded none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The agent has its own search over its whole conversation, so ⌘F on its
    /// pane asks the core where the search goes (`pane_find_open`) rather
    /// than opening Hide's find bar (`agent_find.rs`).
    pub own_find: bool,
    /// The pane this agent was spawned from, as the `parent_pane` token its
    /// spawner declared (Herdr records no lineage). It is a claim that holds
    /// only while both panes still host the sessions it was written for:
    /// `wire.rs` checks the child's, `crate::agent_state::apply_lineage` the parent's and
    /// clears the claim when either moved on, so an agent that took over a
    /// pane is a root rather than a child of whoever the pane's last agent
    /// was spawned by.
    #[serde(skip_serializing)]
    pub spawned_from_pane_id: Option<String>,
    /// The exact pane token before device scoping. It lets a disconnected
    /// cross-device parent resolve again when its machine reconnects.
    #[serde(skip_serializing)]
    pub declared_parent_pane_id: Option<String>,
    /// The stable machine identity recorded for a cross-device parent.
    #[serde(skip_serializing)]
    pub spawned_from_machine_id: Option<String>,
    /// The digest of the parent's session the declaration was written for.
    #[serde(skip_serializing)]
    pub declared_parent_session: Option<String>,
    /// The digest of the session this pane's agent runs now, which a child's
    /// declaration is compared with when this row is its parent.
    #[serde(skip_serializing)]
    pub lineage_session: Option<String>,
    /// Hide's server mark as the pane carries it (`wire::SERVER_MARK_TOKEN`):
    /// which Herdr server a saved `state_change_seq` for this pane can be
    /// compared on.
    #[serde(skip_serializing)]
    pub server_mark: Option<String>,
    /// Ownership, derived from the lineage alone: a root is the operator's own
    /// work, a descendant is work the root delegated. It is the fourth derived
    /// axis beside demand, activity and read, and it is advice rather than a
    /// boundary - a delegated pane is still selectable and still takes input,
    /// because the operator has to be able to read a child's blocked prompt
    /// (PRD D-36, D-56).
    ///
    /// An orphan is a root again: when the parent is gone, ownership comes
    /// back to the person (PRD D-52).
    pub delegated: bool,
    /// The pane that spawned this one, when that pane's agent is still in the
    /// list. `spawned_from_pane_id` records what Herdr said; this records
    /// which row it actually resolved to, so a breadcrumb never points at a
    /// pane that is not there.
    pub lineage_parent_pane_id: Option<String>,
    /// The ancestors between the lineage root and this agent, root first and
    /// excluding this agent. The breadcrumb is this list; nothing persists a
    /// visited path, because a stored one rots across a restart, a tab switch
    /// or a child exiting (PRD D-18).
    pub lineage_path_pane_ids: Vec<String>,
    /// Every agent sharing this agent's parent, in the same order the parent
    /// lists its children, including this agent. It is what a breadcrumb
    /// step's dropdown offers (PRD B10).
    ///
    /// A root has none: the layer above a root is the sidebar, not the
    /// breadcrumb, and independent roots are not one another's siblings.
    pub lineage_sibling_pane_ids: Vec<String>,
    /// What every live descendant of this row is doing, counted by state, for
    /// the badge the row wears while its descendants are folded away. It is
    /// derived on the lineage pass and counts all descendants, not only the
    /// direct children, because a grandchild's question is still this row's
    /// to answer (PRD B3, B4, D-05).
    pub descendant_counts: DescendantCountsSnapshot,
    /// Facts for the common direct-child popover, independent of tree folding.
    pub direct_child_counts: DescendantCountsSnapshot,
    /// What a quiet row, one with no demand of its own that is stopped, idle
    /// or done, is waiting on: a working descendant, a background job its
    /// session proves is still running, or the reply to a Hide request it
    /// sent while the recipient is still making it. A waiting row has not
    /// finished, so it stays in Working rather than reading as finished, and
    /// it reaches Done only once nothing it waits on is in progress
    /// (sidebar-agent-status D-01). It
    /// travels as this additive field beside `group: working`, never as a new
    /// group value, so a decoder that does not know it draws an ordinary
    /// Working row (D-10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait: Option<AgentWait>,
    /// The pull request whose checks this row would wait on: a duty pull
    /// request of its own with checks running on a current read, or one its
    /// session just printed whose read has not landed (`AgentWait::Checks`).
    /// Laid before the lineage (`Runtime::lay_checks`); the line names it.
    #[serde(skip_serializing)]
    pub checks_wait: Option<u32>,
    /// The checks this row waited on finished in its current state, and how
    /// (docs/status-model.md, A quiet row that waits). It is part of the read
    /// fingerprint, so the end turns the row unread once.
    #[serde(skip_serializing)]
    pub checks_ended: Option<ChecksEnded>,
    /// The background tasks a finished turn left running, which its line
    /// names (status-marks D-19); 0 on any other row.
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub background_left: u32,
    /// The demands and completions of this row's live descendants, keyed by
    /// the descendant pane. It is what the read record compares against, so
    /// a descendant asking or finishing turns this row unread the same way
    /// its own state change would (PRD B5, B6, D-03).
    #[serde(skip_serializing)]
    pub descendant_signals: BTreeSet<DescendantSignal>,
    /// Tree-only presentation. The canonical agent list and its read axes stay flat.
    pub lineage_depth: usize,
    pub lineage_child_pane_ids: Vec<String>,
    /// Every live descendant pane in the order a close of this row takes
    /// them: each descendant before its parent, so the row itself would go
    /// last (PRD close-agent-subtree D-20). A row on a device that is not
    /// connected is never listed, because it is neither shown nor closable
    /// (D-16). Absent from the wire when there is none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub close_descendant_pane_ids: Vec<String>,
    pub lineage_root_checkout_id: Option<String>,
    pub lineage_worktree_badge: Option<String>,
    pub lineage_orphan: bool,
    pub lineage_hint: Option<String>,
    pub raised_hint: Option<String>,
    /// The pane the hint points at, when that pane still holds an agent Hide
    /// can name. It is what makes the line something to follow rather than
    /// something to read.
    pub spawn_origin_pane_id: Option<String>,
    pub lineage_collapsed: bool,
    /// Present while Hide has ended this agent's process and holds its
    /// conversation to resume (`agent_sleep.rs`). It rides beside `group`,
    /// never as a group or transport value, and is absent from the wire when
    /// the agent is awake, so an awake row keeps exactly its keys (PRD D-16).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sleep: Option<AgentSleepSnapshot>,
    /// What the label worker read of this agent's session, which the request
    /// block is built from (PRD overview-request-view D-14).
    #[serde(skip_serializing)]
    pub(crate) row_facts: Option<crate::request_view::RowFacts>,
    /// The request and block letters the agent's session sent whose answer
    /// it still waits for, each with who will answer it. Laid by the runtime
    /// from the delivery ledger, never by a session read, so it does not make
    /// `row_facts` present; the lineage pass decides from the recipients
    /// whether one of those answers is still being made.
    #[serde(skip_serializing)]
    pub(crate) awaited_replies: Vec<AwaitedReply>,
    /// Hide AI started this agent (PRD hide-ai-origin-mark D-01). Set by the
    /// runtime from the coordination ledger, so a session read never makes
    /// it; absent from the wire for every other agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hide_ai: Option<HideAiSnapshot>,
    /// The request view's part of the row: the verb and since when, the
    /// request and reply lines, the pull requests (`request_view.rs`).
    /// Absent until the core has built it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<crate::request_view::AgentRequestSnapshot>,
}

/// What a sleeping agent's row and pane say about it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AgentSleepSnapshot {
    /// `sleeping`, `waking` or `failed`.
    pub state: String,
    /// Why the last wake failed, in the operator's words; only on `failed`.
    pub reason: Option<String>,
    /// When the agent last changed state before it slept, for "Resuming the
    /// conversation from <elapsed>".
    pub since_unix_ms: u64,
    /// The agent's last progress sentence, kept from before it slept.
    pub progress: Option<String>,
}

/// Whether the pane menu's Sleep agent can run now, and why not (PRD B15).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AgentSleepActionSnapshot {
    pub available: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WorkspaceSnapshot {
    pub agent_scope: crate::agent_state::Scope,
    pub home_issues: crate::issues::ProjectIssuesSnapshot,
    /// The project's tasks in the source-neutral shape the web reads
    /// (`tasks.rs`); `home_issues` stays as older readers read it.
    pub tasks: crate::tasks::ProjectTasksSnapshot,
    /// The project's pull requests as `gh pr list` answered them, one per
    /// branch, for the Overview's PRs tab (PRD overview-lenses-prs), where a
    /// pull request with no checkout here (a bot's branch) is a row too. A
    /// local Git project's only, and absent from the wire while empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pull_requests: Vec<PullRequestSnapshot>,
    pub id: String,
    pub label: String,
    pub path: String,
    pub remote_target_id: Option<String>,
    pub expanded: bool,
    pub device_id: String,
    pub repo_name: String,
    pub is_git: bool,
    pub default_branch: Option<String>,
    /// Local branch refs available as a base for a new worktree. This is not
    /// derived from checkout rows because a valid base need not be checked
    /// out anywhere.
    pub branches: Vec<String>,
    pub registered: bool,
    pub temporary: bool,
    /// The Herdr workspaces whose panes sit in this project, in Herdr order.
    /// Project identity is the repository path, not a Herdr workspace id, so
    /// Herdr dropping its workspace when the last pane closes leaves the row
    /// and the persisted focus in place. Commands that need any workspace use
    /// the first entry; purpose projection and persistence use the last
    /// checkout occupant because that is the token whose value wins. An empty
    /// list means Herdr has none here yet. Commands may reuse an entry only
    /// while no other project also contains it.
    pub session_workspace_ids: Vec<String>,
    /// The newest activity anywhere in this project, in Unix milliseconds:
    /// the latest of its checkouts' last commit and its agents' last activity,
    /// the same signal `project_context::sort_projects` orders the list by.
    /// Absent when the project has neither, so the row can leave its time
    /// blank instead of claiming the epoch. An additive snapshot field: the
    /// shell reads it when present and keeps its previous behavior when not.
    #[serde(default)]
    pub last_activity_unix_ms: Option<u64>,
    /// Whether the operator pinned this project's registration. A pinned
    /// project sorts before its device's unpinned ones and is exempt from the
    /// device's inactive fold; the shell draws the pinned rows under their own
    /// `Pinned` section. It is the registration's flag carried onto the row
    /// (D-07), so an unregistered workspace is never pinned.
    #[serde(default)]
    pub pinned: bool,
    /// This row is its device's Home (`~/hide`, PRD home-device-rail D-01):
    /// the registration's flag carried onto the row. The shell draws it as
    /// the Home row and never lists or counts it as a project.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_home: bool,
    pub checkouts: Vec<CheckoutSnapshot>,
    /// Checkout rows grouped after the active rows in this project. The full
    /// rows stay in `checkouts`, which remains the authority for focus,
    /// search, tab state, and every non-sidebar consumer.
    pub inactive_checkouts: InactiveCheckoutGroupSnapshot,
    pub session_folds: SessionCheckoutFolds,
    /// What the project's agents are doing, which the sidebar row draws as
    /// its name's weight and its sleeping fold (PRD sidebar-titlebar-refresh
    /// D-02, D-03, D-20).
    pub agent_presence: ProjectAgentPresenceSnapshot,
    /// What `Remove project…` would close, counted by the core so the
    /// confirmation names the same panes the close will send to Herdr
    /// (D-10). Both are zero for a project Herdr has no pane in, which is
    /// the registration-only removal.
    #[serde(default)]
    pub removal: WorkspaceRemovalGateSnapshot,
    /// A local Git project's allocated disk, for the web Overview's facts
    /// line. Absent from the wire until the project has a size, a reason it
    /// has none, or a measurement in flight, so an unmeasured project keeps
    /// exactly its keys.
    #[serde(default, skip_serializing_if = "ProjectDiskSnapshot::is_empty")]
    pub disk: ProjectDiskSnapshot,
    /// The disk cleanup review or run of this project, while there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleanup: Option<crate::live::cleanup::CleanupSnapshot>,
}

/// What a project's agents are doing, decided by the core so the shell only
/// draws it (`project_context::agent_presence`).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ProjectAgentPresenceSnapshot {
    /// An awake agent here draws a mark (turn, going, waiting, done or unread);
    /// quiet, sleeping or no agents leave it false.
    pub working: bool,
    /// Present while the project has a sleeping agent and no awake one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asleep: Option<ProjectAsleepSnapshot>,
}

/// A project whose every agent sleeps: it starts folded each time it falls
/// asleep, and stays open once the operator opens it until an agent wakes
/// (the `asleep/<project>` key of `session_open_folds`, D-19).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ProjectAsleepSnapshot {
    /// When the most recent of its agents fell asleep, in Unix milliseconds.
    pub since_unix_ms: u64,
    pub expanded: bool,
}

/// A Git project's allocated disk: every worktree and the shared Git
/// directory, counted once, as the catalog sums them (`disk_total_bytes`).
/// `total_bytes` is known only once every part is measured; `measuring`
/// says the Projects tab's sweep is measuring it now and `queued` that it
/// will (PRD overview-device-tasks B53), both drawn over the last size.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ProjectDiskSnapshot {
    pub total_bytes: Option<u64>,
    pub unavailable_reason: Option<String>,
    pub measuring: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub queued: bool,
    /// Free space of the project's volume when it was measured; known
    /// whether or not every checkout could be measured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_bytes: Option<u64>,
    /// The size of that volume.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_total_bytes: Option<u64>,
    /// What the checkouts that could be measured add up to. `total_bytes`
    /// stays absent while any checkout is unavailable; this is the subtotal
    /// the entrance may show beside it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmed_bytes: Option<u64>,
    /// The layers of the checkouts that could be measured, summed; absent
    /// only while none was measured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layers: Option<ProjectDiskLayersSnapshot>,
}

/// A project's allocated bytes by layer: the sum over its checkouts, and the
/// shared Git directory on its own.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ProjectDiskLayersSnapshot {
    pub build_cache: u64,
    pub dependencies: u64,
    pub source: u64,
    pub other: u64,
    pub shared_git: u64,
}

impl ProjectDiskSnapshot {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// The counts the project removal confirmation reads (D-10).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct WorkspaceRemovalGateSnapshot {
    pub pane_count: usize,
    /// Panes whose agent is currently working.
    pub running_agent_count: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct InactiveCheckoutGroupSnapshot {
    pub expanded: bool,
    /// IDs in the same recent-activity order as `WorkspaceSnapshot.checkouts`.
    pub checkout_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct SessionCheckoutFolds {
    pub empty: Vec<String>,
    /// Worktrees whose unresolved agents are all delegated children. They get
    /// no checkout line and sit in no fold: the way to a child is the tree of
    /// the root that owns it.
    pub child_only: Vec<String>,
    pub cleanup: Vec<String>,
    pub empty_open: bool,
    pub cleanup_open: bool,
    pub open_prs: usize,
}

/// Agent counts and representative after Hide applies its pane-level read records.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CheckoutAgentSummary {
    pub representative_pane_id: Option<String>,
    pub needs_you: usize,
    pub done: usize,
    pub working: usize,
    pub seen: usize,
    /// A subset of Seen, not an additional group.
    pub unknown: usize,
    /// How many agents here draw each mark on their own row, so a row that
    /// stands for them can say what opening it would show.
    pub marks: MarkCountsSnapshot,
}

/// How many rows draw each status mark (docs/status-model.md, The status
/// badge): the operator's turn, in progress, to read as finished, and to read
/// otherwise. A quiet row draws no mark and is counted in none.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct MarkCountsSnapshot {
    pub turn: u32,
    pub going: u32,
    pub done: u32,
    pub unread: u32,
}

/// The source of the one-line checkout purpose chosen by the core.
///
/// The shell uses this only for presentation tone. Resolution stays here so
/// every surface reads the same fallback order instead of reconstructing it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutPurposeOrigin {
    Token,
    BranchDescription,
    AgentTitle,
    PullRequestTitle,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CheckoutPurposeSnapshot {
    pub text: String,
    pub origin: CheckoutPurposeOrigin,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CheckoutSnapshot {
    pub agent_scope: crate::agent_state::Scope,
    pub issue: Option<crate::issues::IssueLinkSnapshot>,
    /// The key of the task in its project's `tasks` this checkout works on.
    pub task_key: Option<String>,
    /// The other tasks of its project's `tasks` this checkout's pull request
    /// closes, so each of them shows the same pull request (D-07).
    pub closes_task_keys: Vec<String>,
    #[serde(skip_serializing)]
    pub branch_issue: Option<String>,
    /// The commit HEAD pointed at when the catalog built this row, read from
    /// Git's files beside the branch name and so known before the worktree
    /// reader has answered. Core only, and never fresher than the reader's
    /// own value: see `head_sha`.
    #[serde(skip_serializing)]
    pub head_oid: Option<String>,
    pub github: GithubStatusSnapshot,
    pub agent_summary: CheckoutAgentSummary,
    pub id: String,
    pub workspace_id: String,
    pub label: String,
    pub path: String,
    pub branch: Option<String>,
    /// One line chosen in this order: Herdr workspace token, branch
    /// description, representative agent title, pull-request title.
    pub purpose: Option<CheckoutPurposeSnapshot>,
    pub is_worktree: bool,
    /// The core-owned home choice, independent of the Git main worktree.
    pub is_primary: bool,
    pub exists: bool,
    pub temporary: bool,
    /// Whether Herdr has a pane here. A worktree earns a row from git rather
    /// than from a pane, so the row needs this to draw the ones with no
    /// terminal dimmed and offer to start one.
    pub has_panes: bool,
    pub dirty: bool,
    pub changed_file_count: u32,
    pub base_branch: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub added_lines: u32,
    pub removed_lines: u32,
    pub unpushed: Option<UnpushedSnapshot>,
    /// This branch's pull request, absent when it has none or when `gh` could
    /// not say. `GithubStatusSnapshot` on the card is what tells those apart.
    pub pull_request: Option<PullRequestSnapshot>,
    /// The complete worktree row backing the card and both removal menus.
    /// All three surfaces therefore consume one core-owned policy result.
    pub worktree: Option<WorktreeSnapshot>,
    /// The work done here is in the base: Git finds HEAD in it, and the link
    /// record ties the sessions that worked here to a merged pull request and
    /// to none still open. Git alone cannot say this, because a checkout with
    /// no commits of its own is in its base too. A merged pull request on
    /// this checkout's own HEAD is the other proof, which `pull_request`
    /// carries.
    pub landed: bool,
    pub tabs: Vec<TabSnapshot>,
    /// The tab Herdr reports as active in this checkout, or `None` when the
    /// workspace's active tab lives in a sibling checkout. A checkout never
    /// substitutes its first tab for a missing one; an active id Herdr names
    /// but the navigator cannot place is reported as a diagnostic instead.
    pub active_tab_id: Option<String>,
    /// The one ordered tab strip for this checkout: Herdr tabs and file tabs
    /// in the order the operator sees them, which the shell draws as it is
    /// given rather than joining two lists of its own.
    pub strip: Vec<StripTabSnapshot>,
    /// The label the next Herdr tab created here should carry. Decided by the
    /// core from Herdr's raw labels, next to the function that formats them,
    /// so the shell never has to read a number back out of a label it was
    /// given to draw.
    pub next_tab_label: String,
    /// The host's own id of the Herdr workspace that owns this checkout, where
    /// every tab Hide creates here goes (`checkout_owner`); `None` while no
    /// owner is open. Read from the session Hide already holds, never sent to
    /// the shell.
    #[serde(skip_serializing)]
    pub owner_workspace_id: Option<String>,
    /// A device's raw row only: its Herdr workspace's `hide_owner` token,
    /// which the grouping checks against the checkout's folder
    /// (`device_catalog::group`).
    #[serde(skip_serializing)]
    pub owner_mark: Option<String>,
    /// A device checkout whose folder the device's helper has not confirmed:
    /// whether it is a Git checkout is unknown, so an owner cannot be opened
    /// for it until it is (`checkout_owner`).
    #[serde(skip_serializing)]
    pub unconfirmed: bool,
}

/// One entry in a checkout's tab strip.
///
/// It carries identity, kind, and the label to draw. The panes, the dirty
/// mark, and the active mark stay on the snapshots the entry points at: those
/// change on a keystroke, and the strip rides the revisioned navigator
/// section, which must not be resent for every edited character.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StripTabSnapshot {
    /// Strip-wide identity, unique across kinds.
    pub id: String,
    pub kind: StripTabKind,
    /// The Herdr tab id or editor tab id this entry stands for.
    pub source_id: String,
    pub label: String,
    /// Whether this editor entry is the checkout's replaceable preview tab,
    /// which the strip titles in italic. A Herdr entry is never one. It rides
    /// the strip rather than only the editor tab because promotion and
    /// replacement change the slot itself, and both happen once per tab, not
    /// per keystroke.
    pub preview: bool,
    /// The one agent this tab holds, when it holds exactly one, drawn as the
    /// tab's identity where a tab is named: the Recent Panels switcher shows
    /// its name and mark instead of the Herdr label, which for an unnamed tab
    /// is only a number (PRD D-16). A shell-only tab and a tab with several
    /// agents carry `None` and keep their label.
    pub agent_identity: Option<AgentChipSnapshot>,
}

/// The kinds of tab a strip holds.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StripTabKind {
    Herdr,
    File,
    Diff,
    Session,
}

impl StripTabSnapshot {
    pub fn herdr(source_id: impl Into<String>, label: impl Into<String>) -> Self {
        let source_id = source_id.into();
        Self {
            id: format!("herdr:{source_id}"),
            kind: StripTabKind::Herdr,
            source_id,
            label: label.into(),
            preview: false,
            agent_identity: None,
        }
    }

    pub fn file(source_id: impl Into<String>, label: impl Into<String>, preview: bool) -> Self {
        let source_id = source_id.into();
        Self {
            id: format!("file:{source_id}"),
            kind: StripTabKind::File,
            source_id,
            label: label.into(),
            preview,
            agent_identity: None,
        }
    }

    pub fn diff(source_id: impl Into<String>, label: impl Into<String>, preview: bool) -> Self {
        let source_id = source_id.into();
        Self {
            id: format!("diff:{source_id}"),
            kind: StripTabKind::Diff,
            source_id,
            label: label.into(),
            preview,
            agent_identity: None,
        }
    }

    pub fn session(source_id: impl Into<String>, label: impl Into<String>, preview: bool) -> Self {
        let source_id = source_id.into();
        Self {
            id: format!("session:{source_id}"),
            kind: StripTabKind::Session,
            source_id,
            label: label.into(),
            preview,
            agent_identity: None,
        }
    }

    /// The strip entry an editor tab stands behind, so a replaced preview tab
    /// can hand its slot to the tab that took its place.
    pub fn editor(tab: &EditorTabSnapshot) -> Self {
        match tab.kind {
            EditorTabKind::File => Self::file(tab.id.clone(), tab.label.clone(), tab.preview),
            EditorTabKind::Diff => Self::diff(tab.id.clone(), tab.label.clone(), tab.preview),
            EditorTabKind::Session => Self::session(tab.id.clone(), tab.label.clone(), tab.preview),
        }
    }

    /// The Herdr half of a strip, in the order the tabs are given.
    ///
    /// A tab with no id has no identity to key a strip entry by, so it is
    /// dropped rather than given a placeholder. Both the local strip and the
    /// remote projection build their Herdr entries here, so the rule that
    /// decides which tabs earn a slot and what an unlabelled one reads as has
    /// one implementation to change.
    /// The Herdr tabs a checkout draws in its strip.
    ///
    /// A tab holding only delegated children is left out: the canvas keeps
    /// one pane, and a strip slot for every child would put the pile back
    /// where the split used to be (PRD B1).
    pub fn from_herdr_tabs(tabs: &[TabSnapshot]) -> Vec<Self> {
        tabs.iter()
            .filter(|tab| !tab.delegated)
            .filter_map(|tab| {
                Some(Self::herdr(
                    tab.id.clone()?,
                    tab.label.clone().unwrap_or_default(),
                ))
            })
            .collect()
    }
}

/// Preserve the established numeric label when a projection has no newer number.
pub fn tab_number(raw_label: &str, fallback: u32) -> u32 {
    automatic_tab_number(raw_label).unwrap_or(fallback)
}

fn automatic_tab_number(raw_label: &str) -> Option<u32> {
    let trimmed = raw_label.trim();
    trimmed
        .strip_prefix("Tab ")
        .unwrap_or(trimmed)
        .trim()
        .parse()
        .ok()
}

/// A custom name wins; automatic names follow the focused pane's identity.
pub fn display_tab_label(
    raw_label: &str,
    number: u32,
    agent_title: Option<&str>,
    process: Option<&str>,
) -> String {
    let trimmed = raw_label.trim();
    let automatic_number = automatic_tab_number(raw_label);
    if !trimmed.is_empty() && automatic_number.is_none() {
        return trimmed.to_owned();
    }
    agent_title
        .into_iter()
        .chain(process)
        .map(str::trim)
        .find(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Tab {}", automatic_number.unwrap_or(number)))
}

/// Sidebar names use only the operator's name, never a pane's automatic title.
fn sidebar_tab_naming<S: serde::Serializer>(
    naming: &TabNaming,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct SidebarName<'a> {
        manual_name: Option<&'a str>,
        number: u32,
        named_by_operator: bool,
    }
    SidebarName {
        manual_name: naming.manual_name(),
        number: naming.number,
        named_by_operator: naming.operator_name().is_some(),
    }
    .serialize(serializer)
}

/// Why a quiet row is waiting rather than finished or stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWait {
    /// A live descendant is working, or stopped on a background task its
    /// session proves.
    Children,
    /// A background command, monitor or task the agent started is still
    /// running in the process that started it.
    Background,
    /// A Hide request the agent sent still waits for its reply, and the
    /// recipient has not taken it in yet or is still making the answer.
    Reply,
    /// A pull request the row holds the duty of has checks running, or one
    /// its session just printed has not been read from GitHub yet.
    Checks,
}

/// How the checks a row waited on ended, for the pull request `number`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChecksEnded {
    pub number: u32,
    pub passed: bool,
}

/// One answer a row's session waits for: the agent the letter went to, as
/// the rows name it, and whether it has taken the letter in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AwaitedReply {
    pub(crate) recipient_pane_id: String,
    /// The recipient's session digest, as the letter named it.
    pub(crate) recipient_session: Option<String>,
    pub(crate) taken_in: bool,
}

/// The status of one agent as a code, never as a sentence: the screen chooses
/// the word in the operator's language, and `english` is the word for output
/// that is not a screen (the project context an agent reads).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatusCode {
    /// The agent stopped and named what stopped it (the label's `blocked`
    /// end). The wire value keeps the name of the demand it rides on.
    Error,
    Question,
    Approval,
    Working,
    Done,
    Idle,
    Unknown,
    /// Waiting: on delegated children, a background job, a reply or a pull
    /// request's checks (the row is in progress), or, by the label's word, on
    /// something nothing proves (the row is to read).
    Waiting,
    /// The turn stopped before the request was done and nothing is left to
    /// wake the agent.
    Unfinished,
    /// A reported completion no label judged: finished or not is unknown.
    Stopped,
    /// A pane that holds a terminal but no agent.
    Attached,
    Sleeping,
    Waking,
    SleepFailed,
}

impl AgentStatusCode {
    pub fn english(self) -> &'static str {
        match self {
            Self::Error => "Blocked",
            Self::Question => "Question",
            Self::Approval => "Approval",
            Self::Working => "Working",
            Self::Done => "Done",
            Self::Idle => "Idle",
            Self::Unknown => "Unknown",
            Self::Waiting => "Waiting",
            Self::Unfinished => "Unfinished",
            Self::Stopped => "Stopped",
            Self::Attached => "Attached",
            Self::Sleeping => "Sleeping \u{b7} resumes when opened",
            Self::Waking => "Waking\u{2026}",
            Self::SleepFailed => "Sleeping \u{b7} couldn\u{2019}t resume",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TabAgentSnapshot {
    pub state: crate::agent_state::TabState,
    pub agent_kind: String,
    pub mark: crate::agent_state::RowMark,
    pub demand: String,
    pub activity: String,
    pub emphasized: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait: Option<AgentWait>,
    pub status_code: AgentStatusCode,
}

impl From<&SidebarAgentSnapshot> for TabAgentSnapshot {
    fn from(agent: &SidebarAgentSnapshot) -> Self {
        Self {
            state: crate::agent_state::TabState {
                mark_tone: agent.state.mark_tone,
            },
            agent_kind: agent.agent_kind.clone(),
            mark: agent.mark,
            demand: agent.demand.clone(),
            activity: agent.activity.clone(),
            emphasized: agent.emphasized,
            wait: agent.wait,
            status_code: agent.status_code,
        }
    }
}

/// The label to give the next Herdr tab created in a checkout.
///
/// Reads Herdr's raw labels, never the ones `display_tab_label` has already
/// formatted. The two are one convention: that function decides how a tab's
/// number is shown, this one decides which number is free. Deriving the free
/// number from formatted text instead would write the convention down a second
/// time, on the far side of the FFI boundary, where a change to either half
/// breaks the other silently.
///
/// A tab Herdr labels with a bare number, or with the `Tab N` this function
/// hands out and Herdr stores verbatim, holds that number; a tab labelled
/// anything else holds none. The answer is the lowest number no tab holds.
/// Counting only bare numbers made every created tab `Tab 2`.
pub fn next_tab_label<'a>(raw_labels: impl IntoIterator<Item = &'a str>) -> String {
    let used = raw_labels
        .into_iter()
        .filter_map(|label| {
            let label = label.trim();
            let number = label.strip_prefix("Tab ").unwrap_or(label);
            number.trim().parse::<u32>().ok()
        })
        .collect::<BTreeSet<_>>();
    // Bounded rather than an open range: n labels cannot cover n + 1
    // candidates, so a gap always exists in this span and the search is total.
    let number = (1..=used.len() as u32 + 1)
        .find(|candidate| !used.contains(candidate))
        .expect("a set of n numbers leaves one of n + 1 candidates free");
    format!("Tab {number}")
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OverlaySnapshot {
    pub kind: Option<String>,
    pub title: Option<String>,
    pub message: Option<String>,
    pub actions: Vec<OverlayActionSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OverlayActionSnapshot {
    pub id: String,
    pub label: String,
    pub destructive: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabNaming {
    pub focused_pane_id: String,
    pub number: u32,
    pub processes: BTreeMap<String, String>,
    pub raw: String,
    pub automatic: String,
    /// The manual name is one the operator gave through Rename, recorded in
    /// `UiStateSnapshot::operator_tab_names`; a spawn's name, or one given
    /// before Hide recorded them, is not. Always false on a device's tab.
    pub by_operator: bool,
}

impl TabNaming {
    /// The name the tab was given, never an automatic `Tab N`: the sidebar's
    /// tab line shows it whoever gave it.
    pub fn manual_name(&self) -> Option<&str> {
        let raw = self.raw.trim();
        (!raw.is_empty() && automatic_tab_number(raw).is_none()).then_some(raw)
    }

    /// The name the operator gave the tab: what makes it a named tab, which
    /// outlives its agents (docs/UI_BEHAVIOR.md, Agent areas).
    pub fn operator_name(&self) -> Option<&str> {
        self.manual_name().filter(|_| self.by_operator)
    }

    /// Whether `names` records this tab's current manual name as the
    /// operator's.
    pub fn named_by(&self, tab_id: Option<&str>, names: &BTreeMap<String, String>) -> bool {
        tab_id
            .and_then(|id| names.get(id))
            .is_some_and(|name| Some(name.as_str()) == self.manual_name())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TabRenameSnapshot {
    pub request_id: String,
    pub tab_id: String,
    pub label: String,
    pub phase: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TabSnapshot {
    #[serde(rename = "sidebar", serialize_with = "sidebar_tab_naming")]
    pub naming: TabNaming,
    pub agent: Option<TabAgentSnapshot>,
    pub id: Option<String>,
    pub workspace_id: Option<String>,
    pub checkout_id: Option<String>,
    pub label: Option<String>,
    pub empty: bool,
    /// Every agent in this tab is somebody else's delegated child, so the tab
    /// exists only to hold work the operator did not ask to look at. The tab
    /// strip leaves it out; the sidebar and the breadcrumb still reach it
    /// (PRD B1, B4, D-40).
    pub delegated: bool,
    pub panes: Vec<PaneSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PaneSnapshot {
    /// Internal incarnation used to bind a close approval to its terminal.
    #[serde(skip_serializing)]
    pub terminal_id: Option<String>,
    /// Herdr could not restore this terminal. Never inferred from its cwd or agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_terminal_id: Option<String>,
    pub id: String,
    /// The three names a pane can be shown by, in the order the header prefers
    /// them. The core ships the ingredients rather than a chosen title so the
    /// local and remote projections cannot disagree about the ladder, and so
    /// the one that runs it stays testable in the shell.
    pub herdr_label: Option<String>,
    pub terminal_title: Option<String>,
    pub cwd: String,
    /// The status code of the agent in this pane, from the same derivation the
    /// sidebar row uses.
    pub status_code: AgentStatusCode,
    /// Whether closing this pane needs the operator to confirm first. Derived
    /// with the agent row's own value so the header and the core cannot
    /// disagree about it.
    pub requires_close_confirmation: bool,
    /// Whether the core needs a fresh activity status before allowing a close.
    pub requires_close_status_check: bool,
    /// The agent's title, from the same ladder the sidebar row shows, so the
    /// header and the row cannot call one pane two things (PRD D-09).
    pub identity_label: Option<String>,
    pub activity_at_unix_ms: Option<u64>,
    pub fork: PaneForkSnapshot,
    /// The ports listened on from at or below this pane's working directory.
    pub ports: Vec<u16>,
    /// Address-preserving listener endpoints from the same port sample.
    pub servers: Vec<ServerEndpointSnapshot>,
    /// What this pane's agent delegated, or why that is unknown. `None` on a
    /// pane Herdr detected no agent in: a shell, an editor or a log gets
    /// neither chips nor an uninstrumented mark, because there is no agent
    /// there to have children (PRD B22, D-30).
    pub children: Option<PaneChildrenSnapshot>,
    /// The breadcrumb: this pane's ancestors root first, then this pane. It
    /// is empty for a lineage root, which is what leaves its header plain.
    pub lineage_path: Vec<LineageStepSnapshot>,
    /// The sleeping agent this pane holds, when it holds one; the pane body
    /// draws it in place of the terminal (PRD B12, B14, B15).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sleep: Option<AgentSleepSnapshot>,
    /// Whether this pane's agent can be put to sleep from its menu, present
    /// only on a local pane holding an awake agent (PRD B15).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sleep_action: Option<AgentSleepActionSnapshot>,
}

/// Why a session is not connected to Hide.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneConnectionReason {
    /// Codex on its shared server runs its hooks there, not in the pane, so
    /// Hide hears nothing from it (openai/codex#48500). Reopen starts it on
    /// its own server.
    CodexSharedServer,
    /// The hook was installed after this session started, so it never fired.
    StartedBeforeHide,
    /// Hide's hook is missing, unreadable or outdated on the machine: the
    /// fix is the agent's row in Settings, not Reopen.
    SetupNeeded,
}

/// What a Reopen of a not connected pane is doing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PaneReopenSnapshot {
    Pending,
    /// The pane stayed as it was; `reason` names why for the popover's one
    /// line.
    Failed {
        reason: PaneReopenFailure,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneReopenFailure {
    /// Herdr or the agent could not start the session again.
    StartRefused,
    /// The pane or its session id is gone.
    SessionGone,
    /// The machine's Codex has not been read, so the start would not know
    /// whether to leave the shared server.
    CodexUnread,
    /// The agent is working or waiting for the operator, so ending it to
    /// start it again would lose what it is doing.
    AgentBusy,
    /// The agent did not hand the terminal back to its shell, or Herdr
    /// refused the end; nothing was started.
    EndRefused,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct PaneConnectionSnapshot {
    pub connected: bool,
    /// Whether the popover offers Reopen: false for a session that Reopen
    /// cannot fix (`setup_needed`) and for a pane on another device.
    pub can_reopen: bool,
    /// Present exactly when `connected` is false.
    pub reason: Option<PaneConnectionReason>,
    /// Absent while no Reopen was asked or the last one ended well.
    pub reopen: Option<PaneReopenSnapshot>,
}

/// What a pane header says about the work its agent delegated.
///
/// The three shapes it can take are deliberately different screens: chips
/// mean known children, an empty chip list on an instrumented pane means a
/// confirmed "this agent is working alone", and `instrumented: false` means
/// Hide cannot see and says why (PRD B21, B23, B32).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PaneChildrenSnapshot {
    pub instrumented: bool,
    /// The first matching reason from the fixed order, present exactly when
    /// `instrumented` is false. It is never empty and never a guess.
    pub uninstrumented_reason: Option<String>,
    /// The accessible name for the uninstrumented mark, so the symbol never
    /// carries the meaning by itself (PRD B37).
    pub uninstrumented_label: Option<String>,
    /// The reason's stable name. The Settings diagnosis reads it to list the
    /// panes whose session predates the install, so that judgement is made
    /// once here rather than by matching a sentence on two screens (PRD B27,
    /// D-61).
    pub uninstrumented_code: Option<String>,
    /// One chip per pane child, in the lineage's own child order. Only panes:
    /// an in-process subagent has no pane, so it cannot be a chip the
    /// operator clicks into (PRD D-63).
    pub chips: Vec<AgentChipSnapshot>,
    /// Whether Hide hears this pane's session, for the pane header's "Not
    /// connected" chip (PRD settings-cleanup B26 to B31). `None` on an agent
    /// whose session Hide cannot reopen to connect it (Grok, OpenCode, Pi,
    /// omp, Cursor: B19) and on one whose hook is switched off or whose machine
    /// the core has not read yet. Derived from the same observation as
    /// `instrumented` above, never a second detector.
    pub connection: Option<PaneConnectionSnapshot>,
    /// The parent badge, chosen from the pane children by the same priority
    /// the Workspace summary uses. In-process subagents take no part in it.
    pub representative: Option<AgentChipSnapshot>,
    /// In-process subagents, summarised and never added to the chip count.
    pub subagents: SubagentCountsSnapshot,
}

/// One agent in a line of them: a pane header chip, a breadcrumb step's
/// sibling, or an Overview worktree row's agent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AgentChipSnapshot {
    pub tone: crate::agent_state::Tone,
    pub pane_id: String,
    /// The short name the chip shows beside its mark.
    pub label: String,
    /// The checkout the child physically runs in. A pane header compares it
    /// with its own checkout and shows the branch instead of the task title
    /// only when they differ.
    pub checkout_label: Option<String>,
    /// The row's second line: the sentence the group chose, or nothing.
    pub detail: Option<String>,
    /// Whether the status word is drawn beside that sentence.
    pub status_word_visible: bool,
    pub agent_kind: String,
    pub demand: String,
    pub activity: String,
    pub emphasized: bool,
    pub mark: crate::agent_state::RowMark,
    pub status_code: AgentStatusCode,
    /// Whether this child is still delegated work. It lifts only when the
    /// child's parent is gone and the child is a root again.
    pub delegated: bool,
}

/// How many live descendants of a row are in each state the badge draws.
///
/// Every count is a real count over rows the projection holds; a descendant
/// whose activity Herdr reports as unknown is in none of them, because a
/// badge that cannot say what a child is doing has nothing to claim, and the
/// exclusion goes to the diagnostic log instead (PRD B7).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct DescendantCountsSnapshot {
    pub error: u32,
    pub approval: u32,
    pub question: u32,
    pub working: u32,
    pub done: u32,
    /// Descendants Herdr cannot classify. Not drawn; the log carries it.
    #[serde(skip_serializing)]
    pub unknown: u32,
}

/// One thing a descendant is doing that its ancestors are told about: an
/// outstanding demand, or a completion.
///
/// It is the unit the read record keeps per ancestor. A signal that appears
/// turns the ancestor unread; a signal that goes away does not, because a
/// question being answered or a finished child starting new work is not
/// news the operator has to act on (PRD B5, B6).
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct DescendantSignal {
    pub pane_id: String,
    /// `question`, `approval`, `error` or `completed`.
    pub kind: String,
}

/// The in-process subagents a pane's session reports.
///
/// Each count is optional because each is separately knowable. An adapter
/// that cannot observe one leaves it `None`, and `None` draws as unknown
/// rather than as zero (PRD B24, B32, D-53).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct SubagentCountsSnapshot {
    pub working: Option<u32>,
    pub done: Option<u32>,
    pub blocked: Option<u32>,
}

impl SubagentCountsSnapshot {
    /// Whether there is anything at all to draw.
    pub fn is_silent(&self) -> bool {
        self.working.is_none_or(|count| count == 0)
            && self.done.is_none_or(|count| count == 0)
            && self.blocked.is_none_or(|count| count == 0)
    }
}

/// One step of a pane header's lineage breadcrumb.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LineageStepSnapshot {
    pub pane_id: String,
    pub label: String,
    /// That layer's other agents, in the parent's own child order, so the
    /// step's dropdown can offer them without a second traversal. It includes
    /// the step itself, so the current position is visible in the list.
    pub siblings: Vec<AgentChipSnapshot>,
}

/// The TCP listeners the machine has, with where each was started from.
///
/// Attribution to a pane is not decided here: the reader ships what it saw and
/// the projection applies the rule, so the rule stays testable without a
/// server to point it at.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ListeningPortsSnapshot {
    pub entries: Vec<ListeningPortSnapshot>,
    pub unavailable_reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ListeningPortSnapshot {
    pub host: String,
    pub port: u16,
    pub cwd: String,
}

/// A reachable address for one observed TCP listener, before HTTP navigation.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct ServerEndpointSnapshot {
    pub host: String,
    pub port: u16,
}

/// The agent a fork was forked from. Both fields are present only while the
/// source's pane still hosts the session the fork was made from; a source
/// that closed or moved on to another conversation leaves both absent, and
/// the mark then names no pane and opens nothing.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ForkSourceSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// What the pane header needs to know about forking this pane.
///
/// Both facts come from Herdr: whether the pane runs an agent whose own fork
/// command can take its recorded session, and whether this pane is itself the
/// result of such a fork.
///
/// `reason` is set only on a pane whose agent can be forked in principle but
/// not yet, so the menu item that is drawn disabled can say why; an agent that
/// cannot be forked at all, and a pane with no agent, carry neither.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct PaneForkSnapshot {
    pub available: bool,
    pub reason: Option<String>,
    /// Present when this pane is a fork, as its row's `forked_from`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<ForkSourceSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConnectionSnapshot {
    pub kind: String,
    pub state: String,
    pub target_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    Sidebar,
    Terminal,
    RightPanel,
    Pet,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FocusedSnapshot {
    pub surface: Surface,
    pub pane_id: Option<String>,
    /// The last operator `focus_pane` the core applied for each page that
    /// numbers its focus events, oldest page first; see [`OperatorFocusAck`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub operator_focus: Vec<OperatorFocusAck>,
}

/// The newest operator focus a page numbered that the core has applied.
///
/// A page that moves its keyboard focus on the operator's click cannot tell a
/// snapshot older than that click from a real move of focus by the core. The
/// page numbers each operator `focus_pane` it sends, and the snapshot that
/// carries the focused pane also says which of those numbers it already
/// includes, so the page follows the snapshot's pane only once the number it
/// last sent has come back. The number is recorded whether the core moved focus,
/// refused it or found it already there, because the page is waiting on the
/// answer, not on the move.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OperatorFocusAck {
    pub client_id: String,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneLayoutDirection {
    Right,
    Down,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PaneLayoutSnapshot {
    pub workspace_id: String,
    pub tab_id: String,
    pub focused_pane_id: String,
    pub zoomed: bool,
    pub root: PaneLayoutNodeSnapshot,
}

impl Snapshot {
    /// The layout being drawn: the one belonging to the tab that holds the
    /// selected pane. Every tab's layout is carried, so this is a lookup and
    /// never waits for Herdr to name the visible tab again.
    pub fn active_pane_layout(&self) -> Option<&PaneLayoutSnapshot> {
        let pane_id = self.terminal.pane_id.as_deref()?;
        self.pane_layouts
            .iter()
            .find(|layout| layout.pane_ids().contains(&pane_id))
    }
}

impl PaneLayoutSnapshot {
    pub fn pane_ids(&self) -> Vec<&str> {
        let mut pane_ids = Vec::new();
        self.root.collect_pane_ids(&mut pane_ids);
        pane_ids
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaneLayoutNodeSnapshot {
    Pane {
        pane_id: String,
    },
    Split {
        direction: PaneLayoutDirection,
        ratio: f32,
        first: Box<PaneLayoutNodeSnapshot>,
        second: Box<PaneLayoutNodeSnapshot>,
    },
}

impl PaneLayoutNodeSnapshot {
    fn collect_pane_ids<'a>(&'a self, pane_ids: &mut Vec<&'a str>) {
        match self {
            Self::Pane { pane_id } => pane_ids.push(pane_id),
            Self::Split { first, second, .. } => {
                first.collect_pane_ids(pane_ids);
                second.collect_pane_ids(pane_ids);
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TerminalSnapshot {
    pub headers: BTreeMap<String, crate::agent_state::header::Header>,
    pub pane_id: Option<String>,
    pub closed: bool,
    pub exit_code: Option<i32>,
    pub panes: Vec<TerminalPaneSnapshot>,
    /// Creation requests the shell sends keys against (PRD
    /// instant-pane-topology D-11), so it can stop once one is discarded.
    pub input_requests: Vec<InputRequestSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InputRequestSnapshot {
    pub request_id: String,
    pub state: InputRequestState,
    /// The pane a pending request's keys wait for, once it exists and holds
    /// the keyboard: a fork's pane until its agent has started. The shell
    /// keeps sending against the request while that pane has the keyboard.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub held_for: Option<String>,
}

/// Where keys sent against a creation request stand: kept until Herdr
/// answers, following the pane Herdr named, or dropped.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputRequestState {
    Pending,
    Ready,
    Discarded,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct TerminalPaneSnapshot {
    pub pane_id: String,
    pub closed: bool,
    pub exit_code: Option<i32>,
    pub transport_state: String,
    pub transport_message: Option<String>,
    pub transport_generation: u64,
    pub transport_attempt: u64,
    pub transport_last_attempt_at_unix_ms: Option<u64>,
    pub transport_exit_category: Option<String>,
    pub transport_retry_decision: String,
    /// This client only observes the pane and Herdr did not move its view
    /// for the last wheel: another client holds the pane's scrolling. Absent
    /// from the wire while false.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub scroll_held_elsewhere: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EditorSnapshot {
    /// Every open document; with View areas (PRD S7) a tab's `preview` says
    /// whether every display of it is a preview.
    pub tabs: Vec<EditorTabSnapshot>,
    /// With View areas, the document of the front Workspace's active area's
    /// active display, and null while that display cannot show it.
    pub active_tab_id: Option<String>,
    /// The active tab's document for a client without View areas. Always null with View
    /// areas, whose documents ride the delta's `documents` section so a
    /// keystroke re-sends one document rather than the editor.
    pub document: Option<EditorDocumentSnapshot>,
    pub archive_detail: Option<ArchiveDetailSnapshot>,
    /// Files being read on a device before their tabs can show; one per
    /// file, in the order they were asked for.
    pub opening: Vec<EditorOpeningSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EditorOpeningSnapshot {
    pub workspace_id: String,
    pub checkout_id: String,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EditorTabSnapshot {
    pub id: String,
    pub workspace_id: String,
    pub checkout_id: String,
    pub path: String,
    pub label: String,
    pub kind: EditorTabKind,
    /// Which Changes group a diff tab represents. Present only for diff tabs.
    pub diff_committed: Option<bool>,
    /// Whether a Markdown file tab draws its formatting in place (Live) or
    /// shows the source editor. Per tab; a reopened tab starts Live.
    pub markdown_live: bool,
    /// Whether a file or diff tab wraps long lines. Per tab; a new or
    /// reopened tab starts wrapped.
    pub wrap: bool,
    pub dirty: bool,
    /// The checkout's one replaceable preview tab (VS Code's model): opened by
    /// a single click, replaced in place by the next single click, and
    /// promoted to an ordinary tab by a double-click, the first edit, Keep
    /// Open, or a drag. A dirty tab is never replaced. Only a shell with
    /// separate View areas remembers its tabs across a restart.
    pub preview: bool,
    /// Why a View tab restored after a restart has no document: its file is
    /// gone or its device cannot read it. Such a tab only offers Close (B20).
    /// Absent from the wire when there is none, so a restored tab keeps
    /// exactly its keys.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EditorTabKind {
    File,
    Diff,
    Session,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ArchiveDetailSnapshot {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub provider: Option<String>,
    pub unavailable_reason: Option<String>,
    pub events: Vec<ArchiveEventSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ArchiveEventSnapshot {
    pub source_offset: Option<u64>,
    pub role: String,
    pub kind: String,
    pub at_unix_ms: u64,
    pub text: String,
    /// The cards a prompt hook gave the agent in full at this prompt, by
    /// number (PRD project-memory-v2 B13).
    pub memory_received: Vec<u64>,
    /// The cards Memory learned from this event and still keeps, by number.
    pub memory_learned: Vec<u64>,
}

/// Where the first-run agent choice stands (`ui_state.agent_onboarding`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentOnboarding {
    /// This Mac's kit record waits for the operator's answer.
    Pending,
    /// Applied, or never needed.
    Done,
}

/// The first-run wizard's flow (PRD first-run-onboarding D-07): a progress
/// saved under another number belongs to a flow this build does not draw.
pub const FIRST_RUN_VERSION: u32 = 1;

/// Where the first-run wizard stands (`ui_state.first_run`): the step it
/// opens on next, saved so a quit, a reload or a failed apply reopens it there
/// (B6). It exists only on a Mac whose kit record waited for the first-run
/// agent choice, so an existing install never sees the wizard (D-08).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct FirstRun {
    pub version: u32,
    pub step: FirstRunStep,
}

/// The wizard's steps in order; `Done` once the last was finished or skipped.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FirstRunStep {
    Agents,
    Notifications,
    Project,
    Done,
}

impl FirstRunStep {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agents => "agents",
            Self::Notifications => "notifications",
            Self::Project => "project",
            Self::Done => "done",
        }
    }
}

/// A screen whose one-line introduction shows on the first visit only
/// (`ui_state.screen_intros_seen`, PRD first-run-onboarding B43).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenIntro {
    Factory,
}

/// The web shell's color theme (Settings > Appearance). `System` follows the
/// operator's OS appearance live; Dark is the default so an update never
/// brightens the screen on its own (web-design-system-reset D-14).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    System,
    Light,
    #[default]
    Dark,
}

impl ThemePreference {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

/// Whether the sidebar, the Sessions list and the tab bar draw an agent's
/// provider logo before its title (PRD agent-logo-view-mode D-02). Hidden is the
/// quiet default, so an update never changes a screen on its own (D-03).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLogoMode {
    Shown,
    #[default]
    Hidden,
}

impl AgentLogoMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shown => "shown",
            Self::Hidden => "hidden",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "shown" => Some(Self::Shown),
            "hidden" => Some(Self::Hidden),
            _ => None,
        }
    }
}

/// The palette every window draws in (Settings > Appearance > Color theme),
/// independent of the Mode above: Hide is the landing page's Owl Field
/// colors, Neutral the grey chrome. Hide is the default, including for a store
/// written before the choice existed (hide-color-theme D-04).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorTheme {
    #[default]
    Hide,
    Neutral,
}

impl ColorTheme {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hide => "hide",
            Self::Neutral => "neutral",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "hide" => Some(Self::Hide),
            "neutral" => Some(Self::Neutral),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionsProviderFilter {
    #[default]
    All,
    Codex,
    Claude,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SessionsSnapshot {
    pub project_id: Option<String>,
    pub checkout_path: Option<String>,
    pub provider_filter: SessionsProviderFilter,
    pub query: String,
    pub loading: bool,
    pub unavailable_reason: Option<String>,
    pub rows: Vec<SessionRowSnapshot>,
    pub total_session_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SessionRowSnapshot {
    pub id: String,
    pub provider: String,
    pub provider_label: String,
    pub locator: String,
    pub checkout_path: String,
    pub first_human_request: Option<String>,
    pub started_at_unix_ms: Option<u64>,
    pub updated_at_unix_ms: u64,
    pub title: Option<String>,
    pub unavailable_reason: Option<String>,
}

/// Project Memory on the wire: the Project a shell opened Memory for in its
/// lane's Memory tool.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct MemorySnapshot {
    pub project: Option<ProjectMemorySnapshot>,
}

/// One Project's Memory as a shell opened it with `memory_open` (PRD
/// project-memory-v2 B20-B24): the switch, the cards, what analysis needs
/// from the operator, and the card just forgotten with its Undo.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ProjectMemorySnapshot {
    /// The catalog Project the shell named.
    pub workspace_id: String,
    /// Why this Project has no Memory here.
    pub unavailable: Option<MemoryUnavailableSnapshot>,
    /// The first read of the Project's Memory has not answered yet.
    pub loading: bool,
    pub enabled: bool,
    pub consent_required: bool,
    /// Cards currently eligible for injection, excluding merged promotions.
    pub usable_cards: usize,
    /// Live cards against the store's cap.
    pub capacity_reached: bool,
    /// Live cards, the most recently changed first.
    #[serde(serialize_with = "serialize_memory_rows")]
    pub rows: Arc<Vec<MemoryRowSnapshot>>,
    /// What the operator is asked about (B28): cards suggested for a
    /// repository file, then promotions that ended without a pull request.
    pub questions: Vec<MemoryQuestionSnapshot>,
    pub analysis: MemoryAnalysisState,
    /// Only the card admitted by the running check, never the whole backlog.
    pub checking_card: Option<u64>,
    /// The UTC-day budget's next reset, drawn in the viewer's timezone.
    pub check_resume_at_unix_ms: Option<u64>,
    /// The card the last forget took away, which Undo brings back (B24).
    pub forgotten: Option<u64>,
    /// The operator's Memory action still running for this Project; another
    /// waits for it.
    pub pending_action: Option<MemoryAction>,
    pub preparing_hooks: bool,
    pub action_failure: Option<MemoryAction>,
    /// The one card shown in full (`memory_expand`).
    pub expanded: Option<MemoryCardSnapshot>,
    /// The matcher's answer to the shell's `memory_search`.
    pub search: Option<MemorySearchSnapshot>,
}

/// Why a Project's Memory cannot be read.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum MemoryUnavailableSnapshot {
    /// The Project left the catalog.
    Unregistered,
    /// The store could not be read; the reason is in the diagnostic log.
    Unreadable,
}

/// One card of a Project's Memory list (B21).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryRowSnapshot {
    /// `m<number>` names it to agents (`hide memory show`) and to every
    /// Memory event.
    pub number: u64,
    pub title: String,
    pub kind: MemoryCardKind,
    /// Bounded text for each card in an active promotion's disclosure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_preview: Option<String>,
    /// Sources that are the operator's own words.
    pub human_source_count: usize,
    /// Distinct sessions given the card in full (D-18).
    pub provided_session_count: usize,
    /// Reads with `hide memory show`.
    pub pull_count: u64,
    pub updated_at_unix_ms: u64,
    /// The card waits to be checked against the repository again and is not
    /// given to agents meanwhile (`확인 중`).
    pub verifying: bool,
    /// The card's way into the repository's own files while it is on one;
    /// absent for a card that is not, was dismissed or whose pull request
    /// closed unmerged (B30-B32).
    pub promotion: Option<MemoryPromotionSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_comparison: Option<MemoryDocumentSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryPromotionSnapshot {
    pub state: MemoryPromotionState,
    /// The durable attempt's start, never the time the shell first saw it.
    pub started_at_unix_ms: u64,
    /// The agent putting the card in the repository, while it runs.
    pub agent: Option<MemoryPromotionAgentSnapshot>,
    pub pr_number: Option<u64>,
    /// The durable original target, absent when legacy provenance is unknown.
    pub pr_url: Option<String>,
    /// The repository-relative file the card went into, once the agent said.
    pub target_path: Option<String>,
    /// Why GitHub could not say whether an `unconfirmed` attempt's pull
    /// request came, when it gave a reason.
    pub github_failure: Option<GithubFailureCategory>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryPromotionState {
    Running,
    PrOpen,
    /// The card points at its file and is never given in full again.
    Merged,
    /// The agent ended without a pull request; the card is asked about again.
    Failed,
    /// The agent is gone and GitHub could not say whether its pull request
    /// came (gh missing or logged out, or failing reads): the attempt is
    /// still followed, and restoring GitHub settles it.
    Unconfirmed,
    /// The card's decision is not built yet and another branch builds it:
    /// that branch's pull request carries the card into its file.
    Building,
    /// The card's decision is not built and no branch builds it; the card is
    /// asked about once.
    Unbuilt,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryPromotionAgentSnapshot {
    pub name: String,
    /// The pane the agent runs in now, which the name jumps to; absent once
    /// it left its pane.
    pub pane_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryDocumentSnapshot {
    pub authority: hide_memory::DocumentAuthority,
    pub document: String,
    pub document_at_unix_ms: Option<u64>,
    pub operator_at_unix_ms: Option<u64>,
}

impl From<&hide_memory::DocumentComparison> for MemoryDocumentSnapshot {
    fn from(comparison: &hide_memory::DocumentComparison) -> Self {
        Self {
            authority: comparison.authority,
            document: comparison.document.clone(),
            document_at_unix_ms: comparison.document_at_unix_ms,
            operator_at_unix_ms: comparison.operator_at_unix_ms,
        }
    }
}

/// One thing the Memory tab asks the operator (B28-B32).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MemoryQuestionSnapshot {
    DocsDisagree {
        number: u64,
        title: String,
        document: MemoryDocumentSnapshot,
    },
    /// "저장소 문서로 올릴까요?" with the counts that crossed the bar.
    Promote {
        number: u64,
        title: String,
        /// Distinct sessions that carried the operator's own words.
        human_session_count: usize,
        pulled_session_count: usize,
    },
    /// The attempt ended with no pull request, try again: "PR 없이 끝남", or,
    /// when its agent never started, why.
    Failed {
        number: u64,
        title: String,
        start_failure: Option<MemoryStartFailure>,
    },
    /// The card's decision is not built and no branch builds it: "아직 안
    /// 만든 결정", with what the agent saw, until the operator leaves it.
    Unbuilt {
        number: u64,
        title: String,
        reason: Option<String>,
    },
}

/// Why a failed promotion's agent never started, as its spawn answered.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryStartFailure {
    pub code: MemoryStartFailureCode,
    /// The last line the new pane's shell showed for `shell_starting`, at
    /// most [`MemoryStartFailure::DETAIL_CHARS`] characters, or the reason
    /// a worker could not process a card; the other
    /// answers' words are the diagnostic log's (`promotion.start_failed`).
    pub detail: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStartFailureCode {
    /// The new pane's shell stopped in its startup files (a question).
    ShellStarting,
    /// The agent's start ended at the shell without the agent, twice.
    AgentNotStarted,
    CardNotProcessed,
    /// Any other answer: the device, the lock, Herdr's refusal.
    Other,
}

impl MemoryStartFailure {
    pub const DETAIL_CHARS: usize = 160;

    /// The failure a promotion keeps (`hide_memory::Promotion::failure`), a
    /// spawn's refusal code with its detail after a unit separator.
    pub fn of(failure: &str) -> Self {
        let short = |text: &str| {
            let text = text.trim();
            (!text.is_empty()).then(|| text.chars().take(Self::DETAIL_CHARS).collect())
        };
        match crate::coordination::split_refusal(failure) {
            (crate::agent_start::SHELL_STARTING, detail) => Self {
                code: MemoryStartFailureCode::ShellStarting,
                detail: detail.and_then(short),
            },
            ("card_not_processed", detail) => Self {
                code: MemoryStartFailureCode::CardNotProcessed,
                detail: detail.and_then(short),
            },
            (code, _) => Self {
                code: if code == "agent_not_started" {
                    MemoryStartFailureCode::AgentNotStarted
                } else {
                    MemoryStartFailureCode::Other
                },
                detail: None,
            },
        }
    }
}

/// What a card is; the same words the extraction and `hide memory show`
/// use.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCardKind {
    Rule,
    Decision,
    Lesson,
    Fact,
    Preference,
}

/// One card in full, as a row expanded in place shows it (B19, B21).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryCardSnapshot {
    pub number: u64,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_comparison: Option<hide_memory::DocumentComparison>,
    /// Repository-relative files the card rests on (B27).
    pub cited_paths: Vec<String>,
    /// The most recent first.
    pub sources: Vec<MemorySourceSnapshot>,
    /// The text the current one replaced, which the card's Undo restores.
    pub previous: Option<MemoryPreviousSnapshot>,
}

/// Where a card came from: one event of one session.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemorySourceSnapshot {
    /// The runtime's name as the screens write it (`Claude Code`).
    pub provider_label: String,
    /// When the words were said; absent for a source older than the time.
    pub at_unix_ms: Option<u64>,
    /// The operator's own words.
    pub human: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryPreviousSnapshot {
    pub body: String,
    /// When the current text replaced it.
    pub changed_at_unix_ms: u64,
}

/// The cards a `memory_search` matched, best first.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemorySearchSnapshot {
    pub query: String,
    pub numbers: Vec<u64>,
}

/// What a Project's Memory analysis shows the operator (B23, D-27). A
/// failure the operator cannot act on is logged and never shown.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryAnalysisState {
    #[default]
    Idle,
    Analyzing,
    /// Memory is on but Hide AI is off or cannot answer, so no new card is
    /// made; existing cards are still given.
    AiNotReady,
    Checking,
    CheckBudgetExhausted,
}

/// One operator action on a Project's Memory (`memory_action`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryAction {
    Enable,
    Disable,
    Delete,
    Forget,
    Undo,
    /// Start an agent that puts the card in the repository (B30); on a failed
    /// promotion, try again.
    Promote,
    /// "안 올림": not until the card changes (B29).
    Dismiss,
    CardRight,
    DocRight,
}

impl MemoryAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enable => "enable",
            Self::Disable => "disable",
            Self::Delete => "delete",
            Self::Forget => "forget",
            Self::Undo => "undo",
            Self::Promote => "promote",
            Self::Dismiss => "dismiss",
            Self::CardRight => "card_right",
            Self::DocRight => "doc_right",
        }
    }
}

fn serialize_memory_rows<S: serde::Serializer>(
    rows: &Arc<Vec<MemoryRowSnapshot>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    rows.as_ref().serialize(serializer)
}

/// What kind of document an open file is, decided once by the host that
/// read it (`hide_node_link::document`) and drawn by the shell as one view per
/// kind. Adding a kind is one variant there and one case in the shell's
/// switch; nothing else in the shell inspects extensions or bytes.
pub use hide_node_link::document::DocumentKind;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EditorDocumentSnapshot {
    pub path: String,
    pub language: Option<String>,
    pub document_kind: DocumentKind,
    pub contents_utf8: Option<String>,
    /// The content revision (`sha256:<hex>`) the draft is based on: read at
    /// open, moved by each save, and what the next save is checked against.
    /// Present exactly for an editable document.
    pub revision: Option<String>,
    pub dirty: bool,
    /// Why an otherwise editable document takes no edits: its size or its
    /// permissions. The kind, not this field, says a PDF or image is read-only.
    pub readonly_reason: Option<String>,
    pub conflict: Option<EditorConflictSnapshot>,
    /// A save that has not come to a known result yet.
    pub save: Option<EditorSaveSnapshot>,
}

/// A save in progress, or one whose answer was lost (PRD S5.5 B14).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EditorSaveSnapshot {
    /// `saving` while it runs (a newer draft may wait behind it), `waiting`
    /// while it waits for the device's helper to finish connecting, `unknown`
    /// when the answer was lost and the file has not been read back yet, and
    /// `checking` while it is read back. An unknown save blocks the next one.
    /// `refused` when the last save was refused or never sent, and
    /// `not_applied` when an unknown save was read back unchanged: it has not
    /// reached the file yet and may still, so it is not called unsaved.
    pub state: String,
    pub message: Option<String>,
}

/// The right panel's persisted sections. AgentSessions never opens Memory.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RightPanelSection {
    #[default]
    #[serde(alias = "git")]
    Overview,
    Explorer,
    Changes,
    AgentSessions,
    Sessions,
    Memory,
}

impl RightPanelSection {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "overview" | "git" => Some(Self::Overview),
            "explorer" => Some(Self::Explorer),
            "changes" => Some(Self::Changes),
            "agent_sessions" => Some(Self::AgentSessions),
            "sessions" => Some(Self::Sessions),
            "memory" => Some(Self::Memory),
            _ => None,
        }
    }
}

/// Explicit interface languages shared by every client of this core.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub enum InterfaceLanguage {
    #[serde(rename = "en")]
    English,
    #[serde(rename = "ko")]
    Korean,
    #[serde(rename = "zh-CN")]
    SimplifiedChinese,
    #[serde(rename = "ja")]
    Japanese,
}

impl InterfaceLanguage {
    /// The interface language a system's primary language means, by the
    /// rules `web/src/i18n/locale.ts` (`systemLanguage`) resolves it with:
    /// English, Korean and Japanese by their base language, Chinese only in
    /// the Simplified script, and `None` for anything else.
    pub fn from_system(tag: &str) -> Option<Self> {
        let mut subtags = tag.split(['-', '_']);
        let language = subtags.next()?.to_ascii_lowercase();
        match language.as_str() {
            "en" => Some(Self::English),
            "ko" => Some(Self::Korean),
            "ja" => Some(Self::Japanese),
            "zh" => {
                // The script, else the one the region implies when a tag is
                // filled in (`zh-TW` is Traditional, `zh` and `zh-CN` Simplified).
                let next = subtags.next().unwrap_or_default();
                let simplified = if next.len() == 4 {
                    next.eq_ignore_ascii_case("hans")
                } else {
                    !["tw", "hk", "mo"].contains(&next.to_ascii_lowercase().as_str())
                };
                simplified.then_some(Self::SimplifiedChinese)
            }
            _ => None,
        }
    }
}

/// The store retains an invalid value; the wire exposes its English fallback.
fn serialize_interface_language<S: serde::Serializer>(
    value: &Option<serde_json::Value>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value
        .as_ref()
        .map(|value| InterfaceLanguage::deserialize(value).unwrap_or(InterfaceLanguage::English))
        .serialize(serializer)
}

/// Which list the sidebar shows under its head (PRD sidebar-task-lens
/// D-10): the projects tree, or every task of the rail's device.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SidebarLens {
    #[default]
    Projects,
    Tasks,
}

impl SidebarLens {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "projects" => Some(Self::Projects),
            "tasks" => Some(Self::Tasks),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Projects => "projects",
            Self::Tasks => "tasks",
        }
    }
}

/// How the 태스크 list groups (PRD sidebar-task-lens D-11): by state across
/// every project, or one group per project.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskView {
    #[default]
    Status,
    Project,
}

impl TaskView {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "status" => Some(Self::Status),
            "project" => Some(Self::Project),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Project => "project",
        }
    }
}

/// The most items the 보기 slot's ← steps back through; past it the oldest
/// goes first (PRD sidebar-task-lens B47).
pub const DETAIL_HISTORY_LIMIT: usize = 20;

/// One thing the lane's 보기 slot shows (PRD sidebar-task-lens D-50).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DetailItem {
    /// A pull request's card, by catalog project and number.
    PullRequest { workspace_id: String, number: u32 },
    /// A session's record (D-44).
    Record {
        session: crate::links::ledger::SessionIdentity,
    },
}

/// The lane's 보기 slot (PRD sidebar-task-lens D-50): what it opened, oldest
/// first with the one it shows last, and whether the lane draws it rather
/// than its Workspace's tool. The rail keeps a slot for it while anything is
/// open; a stored value this build cannot read loads empty.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DetailSlot {
    #[serde(default)]
    pub items: Vec<DetailItem>,
    #[serde(default)]
    pub shown: bool,
}

impl DetailSlot {
    pub fn top(&self) -> Option<&DetailItem> {
        self.items.last()
    }

    /// Opens `item` in front: the same item again only shows it, another one
    /// is stepped back to by ←, and past the cap the oldest goes.
    pub fn open(&mut self, item: DetailItem) {
        if self.top() != Some(&item) {
            self.items.retain(|held| held != &item);
            self.items.push(item);
            if self.items.len() > DETAIL_HISTORY_LIMIT {
                self.items.remove(0);
            }
        }
        self.shown = true;
    }

    /// ←: the item before the one shown; with none left the slot closes.
    pub fn back(&mut self) {
        self.items.pop();
        self.shown = !self.items.is_empty();
    }

    pub fn close(&mut self) {
        self.items.clear();
        self.shown = false;
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct UiStateSnapshot {
    #[serde(default)]
    pub resolved_sessions: BTreeMap<String, crate::agent_state::sessions::Resolution>,
    /// A new input starts work beyond any PR that had already settled.
    #[serde(default)]
    pub session_resolution_inputs: BTreeMap<String, u64>,
    /// Sessions in a checkout start open; older expansion choices are ignored.
    #[serde(default)]
    pub session_collapsed_checkout_ids: Vec<String>,
    #[serde(default)]
    pub session_open_folds: Vec<String>,
    #[serde(default = "default_panel_visible")]
    pub left_sidebar_visible: bool,
    /// The device rail on the sidebar's left. It is shown by default, and an
    /// older store without the field keeps showing it.
    #[serde(default = "default_panel_visible")]
    pub device_rail_visible: bool,
    #[serde(default = "default_panel_visible")]
    pub right_panel_visible: bool,
    /// Which of the right panel's four sections is showing. Persisted rather
    /// than held in the view, because hiding the panel tears the view down
    /// and the section has to come back the way it was left.
    #[serde(default)]
    pub right_panel_section: RightPanelSection,
    pub expanded_paths: Vec<String>,
    /// Folders expanded in each SSH device's Explorer, by device id; this
    /// machine's are `expanded_paths`. A path means something only on its
    /// own device, so two devices' same path never share a row (S5.5 B2).
    #[serde(default)]
    pub device_expanded_paths: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub collapsed_workspace_ids: Vec<String>,
    /// Sidebar workspaces (checkout paths) whose agent rows the removed
    /// native shell hid; absence is open. It is kept as that shell's own
    /// preference so an older store still loads; the web reads
    /// `expanded_checkout_ids`.
    #[serde(default)]
    pub collapsed_checkout_ids: Vec<String>,
    /// Checkouts whose agent rows the web Projects list opened. Absence is
    /// the default closed state, where the checkout's second line names its
    /// agents, so an old store needs no migration.
    #[serde(default)]
    pub expanded_checkout_ids: Vec<String>,
    /// Projects whose Inactive checkout group the operator opened. Absence is
    /// the default collapsed state, so old stores need no migration.
    #[serde(default)]
    pub expanded_inactive_checkout_project_paths: Vec<String>,
    /// Device groups whose Inactive projects group the operator opened.
    /// Absence is the default collapsed state.
    #[serde(default)]
    pub expanded_inactive_project_device_ids: Vec<String>,
    #[serde(default)]
    pub project_base_branches: BTreeMap<String, String>,
    /// Private choices keyed by machine plus canonical main-worktree root.
    /// Missing fields retain legacy path-based issue/base behavior.
    #[serde(default)]
    pub project_settings: BTreeMap<String, crate::project_settings::ProjectSettings>,
    /// Bounded transient write receipts; never restored or saved as choices.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub project_settings_writes: BTreeMap<String, crate::project_settings::WriteStatus>,
    /// Agent panes whose descendants the operator opened in the sidebar tree.
    /// Absence is the default folded state, so a row with children starts
    /// folded and shows its descendant badge until the operator opens it.
    /// The set is keyed by pane id and lives as long as the pane does: a
    /// Herdr restart mints new ids, so it starts folded again (PRD D-06,
    /// D-11).
    #[serde(default)]
    pub expanded_agent_pane_ids: Vec<String>,
    /// Agent panes whose children the operator opened in the sidebar's
    /// 태스크 list, kept apart from the 프로젝트 tree's (PRD
    /// sidebar-task-lens D-48; the name is the Sessions list's, which held
    /// it before, so a store and a core move read it as they did); folded by
    /// default and pruned when the pane closes.
    #[serde(default)]
    pub sessions_expanded_agent_pane_ids: Vec<String>,
    /// Which list the sidebar shows under its head (PRD sidebar-task-lens
    /// D-10, D-25): the projects at first, then the last one chosen.
    #[serde(default)]
    pub sidebar_lens: SidebarLens,
    /// How the 태스크 list groups its tasks (D-11), kept like the lens.
    #[serde(default)]
    pub task_view: TaskView,
    /// The lane's 보기 slot (D-50): a pull request's card or a session's
    /// record, one per window and no checkout's or Agent tab's, so it stays
    /// when either changes.
    #[serde(default)]
    pub detail: DetailSlot,
    pub selected_path: Option<String>,
    pub selected_pane_id: Option<String>,
    /// Set by a core move's rekey (`node_migration::reown`): the machine the
    /// core now runs on kept its selection only in its own Herdr while the
    /// other core ran, so the first local session opens on Herdr's focus
    /// (`consume_restore_hint`). Core only, and kept in the store until that
    /// session takes it.
    #[serde(skip)]
    pub focus_from_herdr: bool,
    pub shortcut_bindings: BTreeMap<String, String>,
    /// Whether `shortcut_bindings` was already brought across from the removed
    /// native app's state. Persisted, never on the wire: once the import ran, an
    /// empty set is the operator's reset to defaults, not a set to import.
    #[serde(skip)]
    pub shortcut_bindings_imported: bool,
    /// The web shell's own pane chords, command id to chord. They live apart
    /// from `shortcut_bindings` because the two hosts reserve different keys
    /// (Chrome keeps ⌘W), so one host's rebind must never become the other's.
    #[serde(default)]
    pub browser_shortcut_bindings: BTreeMap<String, String>,
    pub pet_visible: bool,
    pub pet_origin: Option<PetOriginSnapshot>,
    pub pet_shortcut: Option<String>,
    #[serde(default)]
    pub focused_device_id: Option<String>,
    #[serde(default)]
    pub focused_checkout_id: Option<String>,
    #[serde(default)]
    pub workspace_registrations: Vec<WorkspaceRegistration>,
    #[serde(default)]
    pub device_registrations: Vec<DeviceRegistration>,
    /// The name the operator registered this machine under while a core on
    /// another machine dialed it, kept when that core moved here, since a
    /// core does not register its own machine (`node_migration::reown`); the
    /// core's own device row carries it as its label, so the machine keeps
    /// its name across the move. None on a machine never registered so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_label: Option<String>,
    #[serde(default = "default_accent_hex")]
    pub accent_hex: String,
    /// The web shell's mode (Light, Dark or System).
    #[serde(default)]
    pub theme: ThemePreference,
    /// Whether list rows and tabs draw the agent's logo (`agent_logo_mode_set`).
    #[serde(default)]
    pub agent_logo_mode: AgentLogoMode,
    /// The palette every window draws in; `color_theme_set` owns it.
    #[serde(default)]
    pub color_theme: ColorTheme,
    /// None follows each client's system language. Raw JSON preserves an
    /// invalid stored selection through unrelated saves without publishing it.
    #[serde(default, serialize_with = "serialize_interface_language")]
    pub interface_language: Option<serde_json::Value>,
    #[serde(default)]
    pub usage: crate::usage::prefs::UsagePreferences,
    /// The first-run agent choice (issue 517), a reading of this Mac's kit
    /// record (the one place that knows whether it was asked and answered):
    /// `None` until the kit has answered once, `Pending` while the record
    /// waits for the operator, `Done` after Apply. A machine that already had
    /// the kit is `Done` without ever asking.
    #[serde(default)]
    pub agent_onboarding: Option<AgentOnboarding>,
    /// The agents the operator left on in the first-run choice (or, for a Mac
    /// that was never asked, the ones it had on), which a device whose record
    /// waits receives once per run.
    #[serde(default)]
    pub agent_onboarding_agents: Vec<String>,
    /// The first-run wizard's progress; `None` on a Mac that was never asked.
    #[serde(default)]
    pub first_run: Option<FirstRun>,
    /// Whether the desktop app shows an OS notification when a root agent
    /// newly needs the operator (Settings > Notifications).
    #[serde(default)]
    pub desktop_notifications: bool,
    /// Done notifications are independent of Needs You; a fresh install
    /// leaves them off. Persistence migrates the former shared switch.
    #[serde(default)]
    pub desktop_notifications_done: bool,
    /// The screens whose one-line introduction was shown once (B43).
    #[serde(default)]
    pub screen_intros_seen: BTreeSet<ScreenIntro>,
    /// The Projects and Agents sidebar's width in CSS pixels, dragged on its
    /// right edge and kept within `SIDEBAR_WIDTH_MIN..=SIDEBAR_WIDTH_MAX`
    /// (PRD sidebar-typography D-08, D-09). A store written before the drag
    /// existed loads at the default.
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: u32,
    /// The interface font size, in points, that the Appearance slider sets.
    /// It scales the shell's own chrome - every `hideFont` call site - and
    /// nothing else. A pane's terminal bytes are sized by `pane_text_scales`
    /// and the editor's code by `editor_text_scale`, so no two of the three
    /// ever apply to the same text.
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    /// Text scale for one pane's own content, keyed by pane id. A pane at the
    /// default scale is absent rather than present at 1.0, so the map stays
    /// the size of what the user actually changed.
    ///
    /// Every key here is a pane id Herdr reports, which is what lets a pane
    /// that goes away take its entry with it. Nothing else may be stored here.
    #[serde(default)]
    pub pane_text_scales: BTreeMap<String, f32>,
    /// Text scale for the file editor's code, which is one surface rather than
    /// one per document.
    ///
    /// It is its own field and not a row in `pane_text_scales` because the
    /// editor is not a pane: keyed into that map it had no pane id to be
    /// reported under, so the pass that drops a departed pane's scale dropped
    /// the editor's zoom on every agent state change.
    #[serde(default = "default_pane_text_scale")]
    pub editor_text_scale: f32,
    /// Eligible agent panes whose terminal is replaced by the local
    /// conversation ledger. This is snapshot-only interaction state;
    /// persistence owns a separate stored representation and intentionally
    /// omits this set. A pane enters it only through `toggle_conversation`:
    /// a new agent pane opens on its terminal, and a pane that leaves the
    /// session leaves the set with it.
    #[serde(default)]
    pub conversation_pane_ids: BTreeSet<String>,
    /// What the operator had already seen on each pane, keyed by pane id.
    ///
    /// This is Hide's own record and the only authority for the read axis.
    /// Herdr marks every pane in a tab seen the moment that tab is focused, so
    /// three finished agents side by side would clear together; a pane-level
    /// record is what keeps them separate. It rides the existing store rather
    /// than a second file (engineering rule 7), and a store written before it
    /// existed loads with an empty record, which reads as everything unread.
    // The store owns persistence; the shell only reads the derived unread axis.
    #[serde(default, skip_serializing)]
    pub pane_read_records: BTreeMap<String, PaneReadRecord>,
    /// The panes whose last stop finished work, keyed by pane id, so a
    /// completion a restored Herdr server no longer reports is still shown
    /// (`agent_state::apply_completions`). The store owns it.
    #[serde(default, skip_serializing)]
    pub pane_completions: BTreeMap<String, PaneCompletion>,
    /// The model and effort Hide started each pane's agent with, keyed by
    /// pane id, so a sleep, a close or a fork captures them for the restart
    /// it makes (`runtime::agent_choice`). The store owns it.
    #[serde(default, skip_serializing)]
    pub pane_launches: BTreeMap<String, PaneLaunch>,
    /// Each agent pane's request-view verb and when it took it, so the time
    /// a row has waited survives a restart (PRD overview-request-view D-40).
    /// The store owns it; the shell reads it on the row.
    #[serde(default, skip_serializing)]
    pub request_verbs: BTreeMap<String, crate::request_view::VerbRecord>,
    /// How long an agent may go untouched before Hide ends its process and
    /// keeps its conversation to resume: 12, 24 or 72 hours, or `None` for
    /// never, the default (PRD D-10).
    #[serde(default)]
    pub agent_sleep_after_hours: Option<u32>,
    /// Each local project's chosen issue source, `github` or `local`, by the
    /// project's path; a project without an entry reads its default source
    /// (`tasks::source_kind`). Set in Settings › Issues.
    #[serde(default)]
    pub project_issue_sources: BTreeMap<String, String>,
    /// How starting work from an issue behaves (Settings › Issues).
    #[serde(default)]
    pub issue_settings: IssueSettingsSnapshot,
    /// The agent kind and each kind's model the operator last started with,
    /// from any start surface (PRD home-device-rail D-18, D-20): the next
    /// start anywhere preselects them.
    #[serde(default)]
    pub agent_start: AgentStartChoice,
    /// Each pane's last state change and last look, and the agents Hide has
    /// put to sleep. Persisted with the rest of this store and never on the
    /// wire: the last look of the tab on screen moves every minute, and a
    /// field that did would resend the navigator to every client.
    #[serde(default, skip_serializing)]
    pub agent_sleep: crate::agent_sleep::AgentSleepStore,
    /// The sessions the operator archived (PRD session-archive). Persisted
    /// with this store and never on the wire: the navigator carries the
    /// rows the shell draws (`archived_sessions`).
    #[serde(default, skip_serializing)]
    pub session_archive: crate::session_archive::SessionArchive,
    /// The checkouts last brought to the front, newest first, at most
    /// `RECENT_CHECKOUT_LIMIT` (PRD cmdk-recent D-05, D-08, D-09). ⌘K reads
    /// the first five it can show; the rest are there so the exclusions of
    /// the front checkout and of Related still leave five. Absent in a store
    /// written before it existed, which loads empty.
    #[serde(default)]
    pub recent_checkouts: Vec<RecentCheckout>,
    /// The terminal panes the keyboard has been in, newest first, at most
    /// `RECENT_PANE_LIMIT`: the order the Agent area's Ctrl+Tab walks (issue
    /// 301). The shell reports each visit with `pane_visit`; a remote pane is
    /// held by its device-scoped id. Absent in a store written before it
    /// existed, which loads empty.
    #[serde(default)]
    pub recent_pane_ids: Vec<String>,
    /// The Home agent pane started as the Factory secretary (PRD
    /// software-factory-ui D-10, B23): 비서에게 묻기 goes to it while it is
    /// listed and starts a new one when it is not. A Herdr restart mints new
    /// pane ids, so the next press starts a new secretary.
    #[serde(default)]
    pub factory_secretary_pane: Option<String>,
    /// The names the operator gave tabs through Rename, by tab id, at most
    /// `OPERATOR_TAB_NAME_LIMIT`: a tab is named, and outlives its agents,
    /// only while its label is still the name recorded here (Herdr keeps tab
    /// ids through a restore). A spawn's name, or one given before this was
    /// recorded, is not the operator's. Persisted and never on the wire.
    #[serde(default, skip_serializing)]
    pub operator_tab_names: BTreeMap<String, String>,
}

/// The most checkouts the recent list keeps; the oldest leave first.
pub const RECENT_CHECKOUT_LIMIT: usize = 10;

/// The most panes the recent pane order keeps; the oldest leave first.
pub const RECENT_PANE_LIMIT: usize = 50;

/// The most tab names the operator's Rename is remembered for; past it, the
/// names of tabs that are gone leave first, and a rename with no room left
/// is not remembered.
pub const OPERATOR_TAB_NAME_LIMIT: usize = 256;

/// One entry of the recent list: the ids that open the checkout and the
/// facts that draw its row while its device has no catalog to read them from.
///
/// No project id is kept: a device project's id changes when its facts
/// resolve, and a checkout id already names the project it is in on this
/// machine, so the shell finds the project from the checkout in the live
/// catalog. The names are the ones the checkout had when it was last in
/// front; a connected device's row is drawn from its catalog instead. No
/// device name is kept: the row's device is a listed one, which the shell
/// names from its row in its own language. A record from an older build
/// still carries a `device_name`, which is read past.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecentCheckout {
    pub device_id: String,
    pub checkout_id: String,
    pub project_name: String,
    /// The checkout's branch, else its label.
    pub branch: String,
}

/// One pane's read mark: the state the operator was looking at the last time
/// the pane held keyboard focus.
///
/// A pane is unread when its current state does not match this record, so a
/// missing record means unread. The session id distinguishes a new agent in a
/// reused pane from the same agent restored by a new Herdr server.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PaneReadRecord {
    #[serde(default)]
    pub state_change_seq: Option<u64>,
    #[serde(default)]
    pub session_id: Option<String>,
    pub demand: String,
    pub activity: String,
    /// Part of the pane-level fingerprint so an idle-to-completed transition
    /// becomes unread even when Herdr's process-local sequence does not move.
    #[serde(default)]
    pub completed: bool,
    /// The descendant demands and completions the operator had seen when this
    /// pane was last read. A descendant signal missing from here is news and
    /// keeps the pane unread; one that has since gone away is trimmed on the
    /// next projection, so the same descendant can be news again after it
    /// works and finishes a second time (PRD B5, B6, B8).
    #[serde(default)]
    pub descendant_signals: BTreeSet<DescendantSignal>,
    /// The server mark (`wire::SERVER_MARK_TOKEN`) of the Herdr server
    /// `state_change_seq` was read on. Herdr's sequence restarts with its
    /// server, so the record is compared with the pane only on that server;
    /// none means no server can be named, as for a record saved before marks.
    #[serde(default)]
    pub server: Option<String>,
    /// The pane no longer matched this record when Hide last judged it, and
    /// the operator has not read it since. Saved whenever it changes, so a
    /// restart that loses Herdr's sequence carries the verdict over rather
    /// than judging the pane again from numbers that no longer compare.
    #[serde(default)]
    pub unread: bool,
    /// How the pull request checks the pane waited on had ended when it was
    /// read. An end the record lacks is news (status-marks D-08); one gone
    /// since is the pane moving on, which its own change already says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks_ended: Option<ChecksEnded>,
}

/// A pane whose last stop finished work, as Hide last saw it. Herdr's
/// restored server can report such a pane as only ready (it does for a
/// restored Claude), and this is what keeps its completion through that.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PaneCompletion {
    /// The digest of the session that finished (`wire::session_digest`).
    #[serde(default)]
    pub session: Option<String>,
    /// The server mark and Herdr sequence the completion was last seen at.
    #[serde(default)]
    pub server: Option<String>,
    #[serde(default)]
    pub state_change_seq: Option<u64>,
}

/// The launch choice of the agent Hide started in a pane, under the name it
/// started it with.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PaneLaunch {
    /// The agent kind, in Herdr's spelling.
    pub kind: String,
    /// The Herdr name Hide started the agent under (a task's, a wake's, a
    /// fork's, a reopen's, or `hide agent spawn --name`). Herdr keeps a name
    /// unique among its live agents, and an agent started in the pane by hand,
    /// or one Herdr gives the pane's id to again, has another name or none,
    /// so the record never describes it.
    pub name: String,
    pub choice: hide_agent_adapter::LaunchChoice,
    /// The conversation the named agent last reported, so the same agent
    /// still matches when it comes back from a Herdr restart without its
    /// name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

/// The scale a pane has until the user zooms it.
pub const DEFAULT_PANE_TEXT_SCALE: f32 = 1.0;

/// One press of the zoom chords. Small enough that the range takes several
/// presses to cross, large enough to be visible in one.
pub const PANE_TEXT_SCALE_STEP: f32 = 0.1;

/// The bounds a pane's text scale is clamped to. Below the minimum the terminal
/// is unreadable; above the maximum a standard pane holds too few columns to
/// show a command line without wrapping.
pub const MIN_PANE_TEXT_SCALE: f32 = 0.7;
pub const MAX_PANE_TEXT_SCALE: f32 = 2.0;

/// Rounded to the step so repeated presses cannot drift the stored value off
/// the ladder through float error.
pub fn clamp_pane_text_scale(scale: f32) -> f32 {
    let stepped = (scale / PANE_TEXT_SCALE_STEP).round() * PANE_TEXT_SCALE_STEP;
    stepped.clamp(MIN_PANE_TEXT_SCALE, MAX_PANE_TEXT_SCALE)
}

pub(crate) fn default_pane_text_scale() -> f32 {
    DEFAULT_PANE_TEXT_SCALE
}

/// How starting work from an issue behaves, chosen in Settings › Issues.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IssueSettingsSnapshot {
    /// Ask the background AI for the new worktree's name; the deterministic
    /// name from the issue's number and title stands until it answers.
    #[serde(default = "default_true")]
    pub ai_worktree_name: bool,
    /// End the first prompt by asking for a pull request that closes the
    /// issue, so the PR and the issue link themselves (`Closes #N`).
    #[serde(default = "default_true")]
    pub closes_instruction: bool,
}

fn default_true() -> bool {
    true
}

impl Default for IssueSettingsSnapshot {
    fn default() -> Self {
        Self {
            ai_worktree_name: true,
            closes_instruction: true,
        }
    }
}

/// The agents Hide starts in a pane itself. `terminal` (a tab alone) is a
/// start choice too, but never a remembered one (PRD home-device-rail D-20).
pub const AGENT_KINDS: [&str; hide_agent_adapter::START_KINDS.len()] =
    hide_agent_adapter::START_KINDS;

/// The agent kind and each kind's model the operator last chose, kept in the
/// core's store so every start surface and every window preselects the same
/// pair (PRD home-device-rail D-18). A kind with no model here starts with
/// its CLI's own default, and no kind at all means the first start of this
/// store.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentStartChoice {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub models: BTreeMap<String, String>,
}

impl AgentStartChoice {
    /// The first choice of a store written before this one existed: the
    /// agent the removed `issue_settings.default_agent` named, when it named
    /// one Hide starts (D-18: that value becomes the first remembered one).
    pub fn seeded(default_agent: Option<&str>) -> Self {
        Self {
            kind: default_agent
                .filter(|kind| AGENT_KINDS.contains(kind))
                .map(str::to_owned),
            models: BTreeMap::new(),
        }
    }
}

/// The Overview's issue work in flight. Each is one slot a newer request
/// replaces, like `task_operation`: the web matches a slot to its own request
/// by id and ignores one it did not ask for.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct IssueWorkSnapshot {
    pub detail: Option<IssueDetailSnapshot>,
    pub name: Option<WorktreeNameSnapshot>,
    pub update: Option<IssueUpdateSnapshot>,
    /// The latest `github_search`; absent from the wire until the first one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<GithubSearchSnapshot>,
    /// Backlog items on their way, the ones that failed, and the newest made
    /// (PRD overview-device-tasks B29, B30, B40, B41, B44, B45).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub backlog: Vec<BacklogItemSnapshot>,
    /// The latest `되돌리기` (B32).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo: Option<BacklogUndoSnapshot>,
    /// Where each item a backlog panel or a Start dialog shows came from, one
    /// per item key, kept while any window shows it (B31, B33).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub origins: Vec<BacklogOriginSnapshot>,
    /// Every backlog item key a missed line made, newest first, for the
    /// backlog row's flag (B27).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missed_keys: Vec<String>,
    /// Every GitHub issue key Hide made for the backlog (the quick add, the
    /// backlog's input or `백로그로`), newest first: only these and memos
    /// show `되돌리기` (D-21, B31).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub made_keys: Vec<String>,
}

/// One backlog item made from the backlog's input, the quick add or a
/// missed line, named by the shell's intent key.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BacklogItemSnapshot {
    pub intent: String,
    pub workspace_id: String,
    pub title: String,
    pub phase: BacklogPhase,
    /// The item's task key once it exists.
    pub task_key: Option<String>,
    /// Why it was not made, on `failed`.
    pub failure: Option<BacklogFailure>,
    /// Whether `backlog_retry` can make it again: a failed item whose job
    /// is kept and whose failure a retry does not repeat.
    pub retry: bool,
    /// The missed line it is made from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missed: Option<crate::links::ledger::MissedSource>,
    pub at_unix_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BacklogPhase {
    Waiting,
    Creating,
    Created,
    Failed,
}

/// The reason line a failed item shows: the four GitHub categories, a queue
/// past its bound, an answer Hide could not confirm, a repository the item
/// may no longer go to, Hide's own record that could not be read just now
/// (worth trying again), and a refusal (final).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BacklogFailure {
    NotInstalled,
    NotLoggedIn,
    NoGithubRemote,
    NetworkOrRateLimit,
    QueueFull,
    Unconfirmed,
    /// The project's repository connection changed while the item was being
    /// made, or a missed line's session did not work in this repository.
    RepositoryChanged,
    /// Hide could not read or reach its own record of the item just now: an
    /// undo of it can be tried again.
    Unavailable,
    Refused,
}

impl From<hide_node_link::gh::GithubFailureCategory> for BacklogFailure {
    fn from(category: hide_node_link::gh::GithubFailureCategory) -> Self {
        use hide_node_link::gh::GithubFailureCategory as Category;
        match category {
            Category::NotInstalled => Self::NotInstalled,
            Category::NotLoggedIn => Self::NotLoggedIn,
            Category::NoGithubRemote => Self::NoGithubRemote,
            Category::NetworkOrRateLimit => Self::NetworkOrRateLimit,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BacklogUndoSnapshot {
    pub task_key: String,
    /// `working`, `done` or `failed`.
    pub phase: String,
    pub failure: Option<BacklogFailure>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BacklogOriginSnapshot {
    pub task_key: String,
    /// `reading`, `ready` or `failed`.
    pub phase: String,
    /// Absent once read when no missed line made the item.
    pub session: Option<crate::links::ledger::SessionIdentity>,
    pub session_title: Option<String>,
    /// The missed line's own words and the assigned task its turn belonged to.
    pub missed_text: Option<String>,
    pub task_name: Option<String>,
    /// The turn's request and answer, for the Start prompt only; read on
    /// this device and never sent to GitHub (D-21).
    pub request: Option<String>,
    pub answer: Option<String>,
}

/// The latest GitHub search (`github_search`), matched by the web's own
/// request id. `query` is the trimmed text the search ran, or was refused
/// with. `results` is empty until `ready`; `message` says why a search
/// `failed`, or that a `ready` one did not cover every repository.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GithubSearchSnapshot {
    pub request_id: String,
    pub query: String,
    /// `working`, `ready` or `failed`.
    pub phase: String,
    pub results: Vec<GithubSearchResult>,
    pub message: Option<String>,
}

/// One pull request or issue a search found: at most 20 of each, pull
/// requests first, in the order the repositories are searched.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GithubSearchResult {
    /// `pr` or `issue`.
    pub kind: String,
    /// `owner/name`.
    pub repository: String,
    pub number: u32,
    pub title: String,
    /// `open`, `closed` or `merged`.
    pub state: String,
    pub url: String,
    /// A pull request's one state ([`PrState::of`]), read with no checks or
    /// review because a search does not ask for them; absent for an issue.
    pub pr_state: Option<PrState>,
}

impl GithubSearchSnapshot {
    pub fn working(request_id: String, query: String) -> Self {
        Self {
            request_id,
            query,
            phase: "working".into(),
            results: Vec::new(),
            message: None,
        }
    }

    pub fn failed(request_id: String, query: String, message: impl Into<String>) -> Self {
        Self {
            phase: "failed".into(),
            message: Some(message.into()),
            ..Self::working(request_id, query)
        }
    }
}

/// One issue as its panel and the Start dialog read it (PRD
/// overview-lenses-issues D-40): the body and, for a GitHub issue, its labels,
/// author, assignees and latest comments. The fields past `message` are
/// empty until `ready`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct IssueDetailSnapshot {
    pub task_key: String,
    /// `reading`, `ready` or `failed`.
    pub phase: String,
    pub body: Option<String>,
    pub message: Option<String>,
    pub labels: Vec<crate::tasks::TaskLabel>,
    pub author: Option<String>,
    pub created_at_unix_ms: Option<u64>,
    pub assignees: Vec<String>,
    pub comment_count: Option<u32>,
    pub comments: Vec<crate::tasks::TaskComment>,
}

impl IssueDetailSnapshot {
    pub fn reading(task_key: String) -> Self {
        Self::settled(task_key, "reading", None)
    }

    /// The answer to a read: `ready` with what it read, or `failed` with why.
    pub fn answered(task_key: String, answer: Result<crate::tasks::TaskDetail, String>) -> Self {
        match answer {
            Ok(detail) => Self::settled(task_key, "ready", Some(detail)),
            Err(message) => Self {
                message: Some(message),
                ..Self::settled(task_key, "failed", None)
            },
        }
    }

    fn settled(task_key: String, phase: &str, detail: Option<crate::tasks::TaskDetail>) -> Self {
        let read = detail.is_some();
        let detail = detail.unwrap_or_default();
        Self {
            task_key,
            phase: phase.into(),
            body: read.then_some(detail.body),
            message: None,
            labels: detail.labels,
            author: detail.author,
            created_at_unix_ms: detail.created_at_unix_ms,
            assignees: detail.assignees,
            comment_count: detail.comment_count,
            comments: detail.comments,
        }
    }
}

/// The answer to a Local issue's title and body edit (`local_issue_update`),
/// matched by the web's own request id.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct IssueUpdateSnapshot {
    pub request_id: String,
    pub task_key: String,
    /// `ready` or `failed`.
    pub phase: String,
    pub message: Option<String>,
}

/// The Overview's pull-request work in flight (PRD overview-lenses-prs):
/// a pull request linked to an issue, and the checks and reviews read for
/// an agent's first prompt. Each is one slot a newer request replaces; the
/// web matches a slot to its own request by `request_id`.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct PrWorkSnapshot {
    pub link: Option<PrLinkSnapshot>,
    pub feedback: Option<PrFeedbackSnapshot>,
    /// The decision cards open now, one per pull request, newest last, at
    /// most [`PR_CARD_LIMIT`] (PRD work-tab-pr-lens D-06).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cards: Vec<PrCardSnapshot>,
}

/// Cards kept at once; the least recently asked for is dropped.
pub const PR_CARD_LIMIT: usize = 8;

/// What a pull request's card reads beyond the list: its body, changed files,
/// unresolved review threads and what a merge may be, per head commit. A part
/// that could not be read is named in `failed` and drawn as
/// `읽지 못함 · 다시 시도` (B52); the others stand.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct PrCardSnapshot {
    pub workspace_id: String,
    pub number: u32,
    /// A read is in flight; the parts below are the last read's.
    pub reading: bool,
    /// The head commit the parts were read at, which a merge is conditioned on.
    pub head: Option<String>,
    pub body: Option<String>,
    pub files: Option<Vec<PrCardFile>>,
    /// How many files the pull request changes in all; more than `files`
    /// holds when GitHub's page was full.
    pub files_total: u32,
    pub threads: Option<Vec<PrCardThread>>,
    pub merge: Option<PrCardMerge>,
    /// `body`, `files` or `threads`, whichever the last read could not get.
    pub failed: Vec<&'static str>,
    /// When the head commit's checks last completed, for the card's
    /// timeline (PRD pr-card-and-session-record-redesign D-15); absent while
    /// unread, none completed or the read lacked it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checks_completed_at_unix_ms: Option<u64>,
    /// When the standing review (the latest approval or change request) was
    /// submitted; absent without one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_at_unix_ms: Option<u64>,
    /// The Merge or branch update asked for from this card, and how it went.
    pub write: Option<PrWriteSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PrCardFile {
    pub path: String,
    pub additions: u32,
    pub deletions: u32,
    /// Marked `linguist-generated` in the project's `.gitattributes`, or a
    /// well-known lock file (D-27).
    pub generated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PrCardThread {
    pub reviewer: Option<String>,
    pub path: String,
    pub line: Option<u32>,
    pub text: String,
}

/// What the repository lets a merge be (D-21).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct PrCardMerge {
    /// The methods the repository allows, in the order Merge offers them.
    pub methods: Vec<hide_node_link::gh::PrMergeMethod>,
    /// The base branch has a merge queue: the button adds to it.
    pub queue: bool,
    /// The signed-in account may write to the repository.
    pub can_merge: bool,
}

/// One GitHub write asked for from a card (D-22, B31-B34).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PrWriteSnapshot {
    /// `merge` or `update_branch`.
    pub action: &'static str,
    /// The head commit the operator was shown.
    pub head: String,
    pub method: Option<hide_node_link::gh::PrMergeMethod>,
    /// `working`, `done` or `failed`.
    pub phase: &'static str,
    /// Why it failed, as a code the card words with what to do:
    /// `head_moved`, `denied`, `conflict`, `not_mergeable`, `unreachable`
    /// or `failed`. The text GitHub gave goes to the diagnostic log.
    pub reason: Option<&'static str>,
}

/// `pr_link_issue`'s progress: the issue made (a new one), the link Hide
/// keeps, then `Closes #N` in the pull request's body. A failed body write
/// keeps what came before it (D-31, D-53); `issue_key` names the issue the
/// pull request is linked to, the new one included, so a retry sends it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PrLinkSnapshot {
    pub request_id: String,
    pub workspace_id: String,
    pub pr_number: u32,
    /// `create`, `link` or `body`: the step working now, or the one that failed.
    pub step: String,
    /// `working`, `ready` or `failed`.
    pub phase: String,
    pub issue_key: Option<String>,
    /// The id the source shows for `issue_key` (`#12`, `L-3`).
    pub issue_id: Option<String>,
    /// The issue was made by this request.
    pub created: bool,
    pub message: Option<String>,
}

/// One failed check of a pull request, by the name GitHub shows and the page
/// that says why.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct PrFailedCheck {
    pub name: String,
    pub url: Option<String>,
}

/// A review that asked for changes, as its author wrote it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PrChangeRequest {
    pub author: Option<String>,
    pub body: String,
}

/// What a pull request says for the work handed on from it: its body, for
/// a new issue made from it (B12), and its failed checks and change
/// requests, for the first prompt of an agent it is handed to (D-46).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PrFeedbackSnapshot {
    pub request_id: String,
    pub device_id: String,
    pub root: String,
    pub pr_number: u32,
    /// `reading`, `ready` or `failed`.
    pub phase: String,
    /// The body once read.
    pub body: Option<String>,
    pub failed_checks: Vec<PrFailedCheck>,
    pub change_requests: Vec<PrChangeRequest>,
    pub message: Option<String>,
}

/// The background AI's name for a worktree started from an issue.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorktreeNameSnapshot {
    /// The web's own id for the request it made.
    pub request_id: String,
    /// `working`, `ready` or `failed`.
    pub phase: String,
    pub name: Option<String>,
    pub message: Option<String>,
}

impl Default for UiStateSnapshot {
    fn default() -> Self {
        Self {
            resolved_sessions: BTreeMap::new(),
            session_resolution_inputs: BTreeMap::new(),
            session_collapsed_checkout_ids: Vec::new(),
            session_open_folds: Vec::new(),
            left_sidebar_visible: true,
            device_rail_visible: true,
            right_panel_visible: true,
            right_panel_section: RightPanelSection::default(),
            expanded_paths: Vec::new(),
            device_expanded_paths: BTreeMap::new(),
            collapsed_workspace_ids: Vec::new(),
            collapsed_checkout_ids: Vec::new(),
            expanded_checkout_ids: Vec::new(),
            expanded_inactive_checkout_project_paths: Vec::new(),
            expanded_inactive_project_device_ids: Vec::new(),
            project_base_branches: BTreeMap::new(),
            project_settings: BTreeMap::new(),
            project_settings_writes: BTreeMap::new(),
            expanded_agent_pane_ids: Vec::new(),
            sessions_expanded_agent_pane_ids: Vec::new(),
            sidebar_lens: SidebarLens::default(),
            task_view: TaskView::default(),
            detail: DetailSlot::default(),
            selected_path: None,
            selected_pane_id: None,
            focus_from_herdr: false,
            shortcut_bindings: BTreeMap::new(),
            shortcut_bindings_imported: false,
            browser_shortcut_bindings: BTreeMap::new(),
            // The pet shows itself on a first run; hiding it is a choice the
            // user makes and the store then remembers (D-09).
            pet_visible: true,
            pet_origin: None,
            pet_shortcut: None,
            focused_device_id: None,
            focused_checkout_id: None,
            workspace_registrations: Vec::new(),
            device_registrations: Vec::new(),
            machine_label: None,
            accent_hex: default_accent_hex(),
            theme: ThemePreference::Dark,
            agent_logo_mode: AgentLogoMode::Hidden,
            color_theme: ColorTheme::Hide,
            interface_language: None,
            usage: Default::default(),
            agent_onboarding: None,
            agent_onboarding_agents: Vec::new(),
            first_run: None,
            desktop_notifications: false,
            desktop_notifications_done: false,
            screen_intros_seen: BTreeSet::new(),
            sidebar_width: default_sidebar_width(),
            font_size: default_font_size(),
            pane_text_scales: BTreeMap::new(),
            editor_text_scale: DEFAULT_PANE_TEXT_SCALE,
            conversation_pane_ids: BTreeSet::new(),
            pane_read_records: BTreeMap::new(),
            pane_completions: BTreeMap::new(),
            pane_launches: BTreeMap::new(),
            request_verbs: BTreeMap::new(),
            agent_sleep_after_hours: None,
            project_issue_sources: BTreeMap::new(),
            issue_settings: IssueSettingsSnapshot::default(),
            agent_start: AgentStartChoice::default(),
            agent_sleep: crate::agent_sleep::AgentSleepStore::default(),
            session_archive: crate::session_archive::SessionArchive::default(),
            recent_checkouts: Vec::new(),
            recent_pane_ids: Vec::new(),
            factory_secretary_pane: None,
            operator_tab_names: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspaceRegistration {
    /// None preserves the root default for registrations written before this choice existed.
    #[serde(default)]
    pub primary_checkout_id: Option<String>,
    pub id: String,
    pub label: String,
    pub path: String,
    /// The node id for this machine. Stores written before node ids are
    /// converted once at launch (`node_migration`), so none is missing.
    pub device_id: String,
    /// Absent in a store written before projects could be pinned, which
    /// loads as unpinned without a warning (D-07). Removing the registration
    /// takes the pin with it; nothing else about the pin is stored.
    #[serde(default)]
    pub pinned: bool,
    /// The device's Home folder (`~/hide`), registered by its first Home
    /// start (PRD home-device-rail D-01, D-04). Absent in every other
    /// registration and in stores written before Home existed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub home: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    try_from = "StoredDeviceRegistration",
    into = "StoredDeviceRegistration"
)]
pub struct DeviceRegistration {
    pub id: String,
    pub label: String,
    pub origin: LinkOrigin,
    /// The Herdr socket on the device, for a host whose server does not
    /// listen at its default path; absent reads the host's default server.
    pub herdr_socket_path: Option<String>,
    /// The operator's one consent for Hide's helper on this device (PRD S5.5
    /// D-20, D-23). Absent until given; a device registered before consent
    /// existed asks before its first file or Git use.
    pub host_consent: Option<HostConsent>,
}

/// How a registered machine's link opens, and so what the core may do to
/// it (PRD core-host-node-remote-core D-04, D-10).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LinkOrigin {
    /// The core dials the machine over SSH under this alias.
    Dialed { ssh_alias: String },
    /// The machine's own node dials this core over the operator's SSH
    /// login; its id is its node id.
    Inbound,
}

impl LinkOrigin {
    pub fn ssh_alias(&self) -> Option<&str> {
        match self {
            Self::Dialed { ssh_alias } => Some(ssh_alias),
            Self::Inbound => None,
        }
    }

    /// Whether the core brings a lost link back itself; a node that dials
    /// in comes back only by dialing again.
    pub fn core_redials(&self) -> bool {
        matches!(self, Self::Dialed { .. })
    }

    /// Whether removing the registration takes Hide's kit off the machine:
    /// a device the core dials, where its helper put it. A node that dials
    /// in runs Hide itself and registers again on its next link, so the kit
    /// the core keeps there through that link stays.
    pub fn removes_kit(&self) -> bool {
        matches!(self, Self::Dialed { .. })
    }

    /// Whether the core's helper there waits for the operator's consent; a
    /// node that dials in was allowed by the operator's own SSH login.
    pub fn takes_consent(&self) -> bool {
        matches!(self, Self::Dialed { .. })
    }

    /// Whether the machine's own node reports its facts (ports, disk,
    /// GitHub) to the core, rather than the core reading them over a dial.
    pub fn reports_own_facts(&self) -> bool {
        matches!(self, Self::Inbound)
    }
}

/// A registration as the store and the shell spell it: the alias of a
/// device the core dials, or the flag of a node that dials in, never both.
#[derive(Deserialize, Serialize)]
struct StoredDeviceRegistration {
    id: String,
    label: String,
    #[serde(default)]
    ssh_alias: Option<String>,
    #[serde(default)]
    herdr_socket_path: Option<String>,
    #[serde(default)]
    host_consent: Option<HostConsent>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    inbound: bool,
}

impl TryFrom<StoredDeviceRegistration> for DeviceRegistration {
    type Error = String;

    fn try_from(stored: StoredDeviceRegistration) -> Result<Self, String> {
        let origin = match (stored.ssh_alias, stored.inbound) {
            (Some(ssh_alias), false) => LinkOrigin::Dialed { ssh_alias },
            (None, true) => LinkOrigin::Inbound,
            (Some(_), true) => {
                return Err(format!(
                    "device {} has both an SSH alias and a node that dials in",
                    stored.id
                ));
            }
            (None, false) => {
                return Err(format!(
                    "device {} has neither an SSH alias nor a node that dials in",
                    stored.id
                ));
            }
        };
        Ok(Self {
            id: stored.id,
            label: stored.label,
            origin,
            herdr_socket_path: stored.herdr_socket_path,
            host_consent: stored.host_consent,
        })
    }
}

impl From<DeviceRegistration> for StoredDeviceRegistration {
    fn from(registration: DeviceRegistration) -> Self {
        let inbound = matches!(registration.origin, LinkOrigin::Inbound);
        Self {
            id: registration.id,
            label: registration.label,
            ssh_alias: registration.origin.ssh_alias().map(str::to_owned),
            herdr_socket_path: registration.herdr_socket_path,
            host_consent: registration.host_consent,
            inbound,
        }
    }
}

pub use hide_node_link::device::HostConsent;

pub(crate) fn default_accent_hex() -> String {
    "#B9FF66".to_owned()
}

/// The narrowest and widest sidebar the drag lands on (`--size-sidebar-min`
/// and `--size-sidebar-max` in design/tokens.json); a width outside is refused.
pub const SIDEBAR_WIDTH_MIN: u32 = 220;
pub const SIDEBAR_WIDTH_MAX: u32 = 440;

/// The sidebar's width until the operator drags it (`--size-sidebar-ideal`).
pub(crate) fn default_sidebar_width() -> u32 {
    292
}

pub(crate) fn sidebar_width_fits(width: u32) -> bool {
    (SIDEBAR_WIDTH_MIN..=SIDEBAR_WIDTH_MAX).contains(&width)
}

pub(crate) fn default_font_size() -> f32 {
    13.0
}

pub(crate) fn default_panel_visible() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EditorConflictSnapshot {
    /// The revision the draft was based on.
    pub opened_revision: String,
    /// What the file holds now; absent when it was removed or could not be
    /// read.
    pub disk_revision: Option<String>,
}

/// One checkout's Git working-tree state, plus the diff of the file the user
/// selected in the changes view. Produced by [`crate::changes`] outside the
/// runtime mutex and ingested whole.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ChangesSnapshot {
    /// The checkout these entries describe. A view that renders entries under
    /// a different root than it asked about would be lying about whose
    /// changes it is showing, so the root travels with them.
    pub root_path: Option<String>,
    /// The working tree's own changes: what `git status` reports.
    pub entries: Vec<ChangedFileSnapshot>,
    /// What commits on this branch changed since [`Self::base_branch`].
    /// Separate from `entries` because they answer different questions - what
    /// is not saved yet, and what this branch is - and the view shows them as
    /// two groups for that reason.
    pub committed: Vec<ChangedFileSnapshot>,
    /// What the committed group is measured against. Absent for a plain
    /// folder and for a repository whose base could not be resolved, in which
    /// case the committed group is not shown at all.
    pub base_branch: Option<String>,
    pub selected_path: Option<String>,
    /// Which group the selection is in. A path can appear in both groups and
    /// its two diffs are different, so the group is part of the selection.
    pub selected_committed: bool,
    pub diff: Option<ChangedFileDiffSnapshot>,
    /// The diff of every visible View display that shows one (PRD S7 A5):
    /// one per distinct file and group among the front Workspace's
    /// area-active displays. Empty, and absent from the wire, in a shell
    /// without View areas.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub diffs: Vec<ViewDiffSnapshot>,
    /// Why there is nothing to list. Present whenever the reader could not
    /// produce entries, so an empty list is never mistaken for "no changes".
    pub unavailable_reason: Option<String>,
    /// The reason above is only that Git finds no repository for this
    /// folder: nothing for the operator to repair, so the Explorer draws no
    /// mark for it and the reason goes to the diagnostic log (issue 570).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub not_a_repository: bool,
    /// Why the latest read of this checkout failed while the entries above,
    /// from the last read that succeeded, are still shown: they may be out of
    /// date, and the next read replaces them (S5.5 B22).
    pub stale_reason: Option<String>,
}

/// The four working-tree states this round presents. Git's porcelain codes
/// carry more distinctions than the view uses; [`ChangedFileStatus::from_porcelain`]
/// is the single place they collapse.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangedFileStatus {
    Modified,
    Added,
    Deleted,
    Untracked,
    Renamed,
    Conflict,
}

impl ChangedFileStatus {
    /// Maps one porcelain v1 `XY` pair onto the presented status. Index and
    /// worktree columns are read together: a file staged as added and then
    /// edited is still an addition to the reader, and a delete on either side
    /// is a delete.
    pub fn from_porcelain(code: &str) -> Self {
        let mut characters = code.chars();
        let index = characters.next().unwrap_or(' ');
        let worktree = characters.next().unwrap_or(' ');
        if matches!(code, "DD" | "AU" | "UD" | "UA" | "DU" | "AA" | "UU") {
            return Self::Conflict;
        }
        if index == '?' && worktree == '?' {
            return Self::Untracked;
        }
        if index == 'R' || worktree == 'R' {
            return Self::Renamed;
        }
        if index == 'D' || worktree == 'D' {
            return Self::Deleted;
        }
        if index == 'A' || worktree == 'A' {
            return Self::Added;
        }
        Self::Modified
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Modified => "modified",
            Self::Added => "added",
            Self::Deleted => "deleted",
            Self::Untracked => "untracked",
            Self::Renamed => "renamed",
            Self::Conflict => "conflict",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ChangedFileSnapshot {
    /// Absolute, so activating a row needs no second join against the root.
    pub path: String,
    /// Relative to the checkout root, which is what the row shows.
    pub relative_path: String,
    /// The source side of a rename, relative to the checkout root. Absent for
    /// every other status. The destination remains `relative_path`, so
    /// opening a row always addresses the file that exists now.
    pub previous_relative_path: Option<String>,
    pub status: ChangedFileStatus,
    /// Lines added and removed in this file. Absent for a file git cannot
    /// count - an untracked file has no index side and a binary file has no
    /// lines - so the row shows no numbers rather than a misleading zero.
    pub added_lines: Option<u32>,
    pub removed_lines: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ChangedFileDiffSnapshot {
    pub path: String,
    pub text: String,
    /// Set when the diff was cut short, naming the limit that cut it. A
    /// silently truncated diff would read as a complete one.
    pub notice: Option<String>,
}

/// A diff a View display shows: the file, the group it is taken in, and the
/// text, bounded like every diff.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ViewDiffSnapshot {
    pub path: String,
    pub committed: bool,
    pub text: String,
    /// Set when the diff was cut short, or when the file is no longer in
    /// its group, naming why.
    pub notice: Option<String>,
}

/// What a branch's pull request is, reduced to the five values the row badge
/// and the card show. The mapping from `gh`'s `state`/`reviewDecision`/
/// `isDraft` triple lives in [`crate::github`]; nothing downstream re-derives
/// it, so the badge cannot drift between the row and the card.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestBadge {
    Merged,
    Closed,
    /// Open, not a draft, and a review decision has been recorded. The three
    /// decisions are one badge with three colours rather than three badges.
    Review,
    /// Open with no review decision, or open as a draft.
    Open,
}

impl PullRequestBadge {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Merged => "merged",
            Self::Closed => "closed",
            Self::Review => "review",
            Self::Open => "open",
        }
    }

    /// The two states a worktree may be removed from. Everything else means
    /// work is still live on the branch, so the card offers no button at all.
    pub fn is_settled(self) -> bool {
        matches!(self, Self::Merged | Self::Closed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    ReviewRequired,
    ChangesRequested,
    Approved,
}

/// CI rollup. Unknown and absent checks must never look like a pass.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestChecks {
    #[default]
    Unknown,
    None,
    Pending,
    Failed,
    Passing,
}

/// The one state every PR mark draws (docs/status-model.md, Pull request
/// visual states), worst first: a checkout's pull request and an agent row's
/// own pull requests take it from [`PrState::of`] and nothing else decides it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    /// Checks failed, or a reviewer asked for changes.
    Failed,
    /// Checks running, absent or unknown, or a review still asked for.
    Pending,
    /// Checks passed, and the review is approved or not asked for.
    Mergeable,
    /// Open as a draft, whatever its checks say.
    Draft,
    Merged,
    Closed,
}

impl PrState {
    /// merged > closed > draft > failed > mergeable > pending. Absent or
    /// unknown checks never read as a pass.
    pub fn of(
        badge: PullRequestBadge,
        is_draft: bool,
        checks: PullRequestChecks,
        review: Option<ReviewDecision>,
    ) -> Self {
        match badge {
            PullRequestBadge::Merged => Self::Merged,
            PullRequestBadge::Closed => Self::Closed,
            PullRequestBadge::Open | PullRequestBadge::Review => {
                if is_draft {
                    Self::Draft
                } else if checks == PullRequestChecks::Failed
                    || review == Some(ReviewDecision::ChangesRequested)
                {
                    Self::Failed
                } else if checks == PullRequestChecks::Passing
                    && matches!(review, None | Some(ReviewDecision::Approved))
                {
                    Self::Mergeable
                } else {
                    Self::Pending
                }
            }
        }
    }
}

/// One pull request GitHub listed. Which checkout it belongs to is decided
/// by `github::pull_request_for_checkout`, never by comparing branch names
/// where a call site stands. Its wire form adds the derived `state`
/// (the `Serialize` impl below), so no shell derives it again.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct PullRequestSnapshot {
    pub closing_issues: Vec<crate::issues::IssueReference>,
    pub title: String,
    pub checks: PullRequestChecks,
    pub number: u32,
    pub head_branch: String,
    pub base_branch: String,
    pub url: String,
    pub badge: PullRequestBadge,
    /// Present only for a `review` badge, and only to pick its colour.
    pub review: Option<ReviewDecision>,
    pub is_draft: bool,
    pub merged_at_unix_ms: Option<u64>,
    pub updated_at_unix_ms: Option<u64>,
    /// When GitHub made the pull request, which ties it to the session whose
    /// tool printed its address then (PRD overview-request-view D-31).
    pub created_at_unix_ms: Option<u64>,
    /// When it was closed or merged, for a row's chip after the request.
    pub closed_at_unix_ms: Option<u64>,
    /// The commit its head branch pointed at when GitHub last saw it. A
    /// settled pull request belongs to a checkout only at exactly this
    /// commit, because a branch name can be used again for new work. Core
    /// only: no shell draws or compares it.
    pub head_oid: Option<String>,
    /// The head branch lives in another repository (a fork), so its name says
    /// nothing about this repository's branch of the same name.
    #[serde(default)]
    pub cross_repository: bool,
    /// What the PRs tool's list groups by (PRD work-tab-pr-lens D-08):
    /// the GitHub values the 5-minute read adds, none of them parsed from a
    /// body or a repository's own script (D-05).
    #[serde(default)]
    pub work: PullRequestWork,
}

/// How a pull request stands against its base, GitHub's `mergeStateStatus`
/// (PRD work-tab-pr-lens D-20). A value this build does not know reads as
/// `Unknown`, which keeps the pull request where it was.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeState {
    #[default]
    Unknown,
    Clean,
    HasHooks,
    /// Merge is allowed but a check that is not required failed.
    Unstable,
    Dirty,
    Behind,
    Blocked,
    Draft,
}

impl MergeState {
    pub fn parse(value: &str) -> Self {
        match value.to_ascii_uppercase().as_str() {
            "CLEAN" => Self::Clean,
            "HAS_HOOKS" => Self::HasHooks,
            "UNSTABLE" => Self::Unstable,
            "DIRTY" => Self::Dirty,
            "BEHIND" => Self::Behind,
            "BLOCKED" => Self::Blocked,
            "DRAFT" => Self::Draft,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewRequestKind {
    User,
    Team,
}

/// One person or team a review was asked of.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ReviewRequest {
    pub kind: ReviewRequestKind,
    pub name: String,
}

/// Whether a review was asked of the signed-in account.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAsk {
    /// The question that tells was not answered, so the pull request may or
    /// may not have been asked of the account or its team. A pull request
    /// starts here and leaves it only when an answer places it.
    #[default]
    Unknown,
    /// Asked of neither the account nor a team of it.
    None,
    /// Asked of the account itself.
    Direct,
    /// Asked of a team the account is in and not of the account.
    Team,
}

/// A pull request's place in its base branch's merge queue, as far as GitHub
/// has said.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "position")]
pub enum MergeQueuePlace {
    /// The queue question was not answered. A pull request starts here.
    #[default]
    Unknown,
    NotQueued,
    /// From 1.
    At(u32),
}

/// The questions a project's pull request read asks beside the list itself
/// (issue #1189). The list alone decides whether the read succeeds; one of
/// these failing leaves the judgments derived from it unknown.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtraQuery {
    /// The signed-in account (`gh api user`): which pull requests are mine.
    Account,
    /// The review requests made to the account (`gh pr list --search
    /// review-requested:@me`): which are asked of me or my team.
    ReviewRequests,
    /// The merge queue place and review request time (`gh api graphql`).
    MergeQueue,
}

/// The GitHub values the PRs tool's list and the card read beyond the
/// badge, checks and review: see [`PullRequestSnapshot::work`].
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct PullRequestWork {
    pub author: Option<String>,
    pub author_is_bot: bool,
    /// The signed-in account wrote it; `None` while the account is unknown.
    pub mine: Option<bool>,
    pub merge_state: MergeState,
    pub review_requests: Vec<ReviewRequest>,
    pub asked_of_me: ReviewAsk,
    /// The checks of an open pull request that failed, by name, and how many
    /// are still running: the second line's `✗ <name>` and `CI · N`.
    pub failed_checks: Vec<PrFailedCheck>,
    pub pending_checks: u32,
    /// The open head's complete GitHub rollup; absent when it was not read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_counts: Option<PrCheckCounts>,
    /// How many of those checks have reported and still run (`QUEUED`,
    /// `IN_PROGRESS`, `WAITING`, `PENDING`, `REQUESTED`), leaving out a
    /// required check that never reported (`EXPECTED`): only a running one
    /// holds an agent's row in progress (status-marks D-07). Core only.
    #[serde(skip)]
    pub running_checks: u32,
    /// Its place in the base branch's merge queue.
    pub queue: MergeQueuePlace,
    /// When the latest review request on it was made, which `리뷰 · N시간째`
    /// counts from.
    pub review_requested_at_unix_ms: Option<u64>,
    /// The sessions the link record says made or worked on it, and the
    /// checkouts (paths) they ran in. Core only: the runtime attaches them
    /// from the record (`runtime/links.rs`), the grouping reads them.
    #[serde(skip)]
    pub sessions: Vec<String>,
    #[serde(skip)]
    pub checkouts: Vec<String>,
}

/// Disjoint counts from one GitHub status-check rollup, never inferred from its summary.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct PrCheckCounts {
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    pub running: u32,
    pub waiting: u32,
    pub skipped: u32,
    pub unknown: u32,
}

impl PullRequestSnapshot {
    pub fn state(&self) -> PrState {
        PrState::of(self.badge, self.is_draft, self.checks, self.review)
    }
}

impl Serialize for PullRequestSnapshot {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        // The head commit, the repository and the closed time stay off the
        // wire: only the core compares them. The created time is the card's
        // `PR 열림` milestone (PRD pr-card-and-session-record-redesign B5).
        let mut out = serializer.serialize_struct("PullRequestSnapshot", 15)?;
        out.serialize_field("closing_issues", &self.closing_issues)?;
        out.serialize_field("title", &self.title)?;
        out.serialize_field("checks", &self.checks)?;
        out.serialize_field("number", &self.number)?;
        out.serialize_field("head_branch", &self.head_branch)?;
        out.serialize_field("base_branch", &self.base_branch)?;
        out.serialize_field("url", &self.url)?;
        out.serialize_field("badge", &self.badge)?;
        out.serialize_field("review", &self.review)?;
        out.serialize_field("is_draft", &self.is_draft)?;
        out.serialize_field("merged_at_unix_ms", &self.merged_at_unix_ms)?;
        out.serialize_field("created_at_unix_ms", &self.created_at_unix_ms)?;
        out.serialize_field("updated_at_unix_ms", &self.updated_at_unix_ms)?;
        out.serialize_field("state", &self.state())?;
        out.serialize_field("work", &self.work)?;
        out.end()
    }
}

pub use hide_node_link::gh::GithubFailureCategory;

/// How a repository's `gh` lookup is doing, independent of what it found.
///
/// "No pull request on this branch" and "the lookup failed" are different
/// answers and the card must not show one as the other, so availability,
/// staleness, and the reason travel beside the results rather than being
/// inferred from an empty list.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct GithubStatusSnapshot {
    pub failure_category: Option<GithubFailureCategory>,
    /// `gh` is installed and logged in.
    pub available: bool,
    /// No lookup has completed yet for this repository.
    pub loading: bool,
    /// The last lookup failed, so the values shown are the previous ones.
    pub stale: bool,
    pub last_success_at_unix_ms: Option<u64>,
    /// The card's one allowed sentence: gh missing, logged out, or the exact
    /// failure. Absent when the lookup is healthy.
    pub unavailable_reason: Option<String>,
    /// The extra questions of the last read that failed while the pull request
    /// list was read: the list is current and the judgments derived from these
    /// are unknown (issue #1189).
    #[serde(default)]
    pub unread_queries: Vec<ExtraQuery>,
}

/// One repository's pull requests as `gh` reported them.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct GithubProjectSnapshot {
    /// Successful component payloads, including a successful empty answer.
    /// Internal reader provenance, not a snapshot wire field.
    #[serde(skip)]
    pub pull_requests_read: bool,
    #[serde(skip)]
    pub issues_read: bool,
    pub issues: crate::issues::ProjectIssuesSnapshot,
    /// The repository's main worktree, which is what identifies a project.
    pub root_path: String,
    pub status: GithubStatusSnapshot,
    /// The account `gh` is signed in as, which "mine" is judged against;
    /// absent before the first successful read.
    #[serde(default)]
    pub viewer: Option<String>,
    /// One entry per branch that has a pull request.
    pub pull_requests: Vec<PullRequestSnapshot>,
}

/// How a project's read ended, as the re-read clock counts it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GithubReadOutcome {
    /// The pull requests and the issues were read.
    Read,
    /// The pull requests were read and the issues were not. The re-read
    /// clock backs off as from a failure, so issues that fail every time
    /// (a fork with its issues turned off) are not asked for every minute,
    /// while the pull requests the answer holds are current.
    IssuesFailed,
    /// The pull requests were not read.
    Failed,
    /// The repository has no GitHub remote: there is nothing to ask GitHub,
    /// and nothing changes that until its remotes do.
    NoRemote,
}

impl GithubReadOutcome {
    /// Whether the answer holds the project's pull requests as GitHub has
    /// them now.
    pub fn pull_requests_read(self) -> bool {
        matches!(self, Self::Read | Self::IssuesFailed)
    }
}

impl GithubProjectSnapshot {
    pub fn read_outcome(&self) -> GithubReadOutcome {
        if self.status.failure_category == Some(GithubFailureCategory::NoGithubRemote) {
            GithubReadOutcome::NoRemote
        } else if !self.pull_requests_read {
            GithubReadOutcome::Failed
        } else if !self.issues_read {
            GithubReadOutcome::IssuesFailed
        } else {
            GithubReadOutcome::Read
        }
    }
}

/// Every repository's pull-request state, keyed by main worktree path.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct GithubSnapshot {
    pub projects: Vec<GithubProjectSnapshot>,
}

impl GithubSnapshot {
    pub fn project(&self, root_path: &str) -> Option<&GithubProjectSnapshot> {
        self.projects
            .iter()
            .find(|project| project.root_path == root_path)
    }
}

impl CheckoutSnapshot {
    /// The commit this checkout is on: the one fact that tells a branch's
    /// current work from an older one of the same name.
    ///
    /// The worktree reader's value wins when it has one, because the reader
    /// runs again whenever a watched HEAD or ref changes, while `head_oid` is
    /// only as new as the row's last build (a topology change or the catalog's
    /// 30-second refresh). Until the reader has answered, and for a checkout
    /// it never listed, the commit read from Git's files stands in, so a
    /// settled pull request is connected from the first frame after a restart
    /// instead of after the reader's first full pass.
    pub fn head_sha(&self) -> Option<&str> {
        self.worktree
            .as_ref()
            .and_then(|worktree| worktree.head_sha.as_deref())
            .or(self.head_oid.as_deref())
    }
}

/// How far a branch is from the remote it tracks. Absent entirely when the
/// branch has no upstream, because "nothing to push" and "nowhere to push to"
/// are different facts and the card shows only the first.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct UnpushedSnapshot {
    pub remote: String,
    pub count: u32,
}

/// One worktree of one repository, as `git worktree list` reports it plus the
/// counts the row badge and the card show.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct WorktreeSnapshot {
    pub lock_reason: Option<String>,
    pub ignored_repositories: Vec<String>,
    pub ignored_scan_unavailable: Option<String>,
    pub head_sha: Option<String>,
    pub last_commit_subject: Option<String>,
    pub last_commit_unix_seconds: Option<u64>,
    pub nested: bool,
    pub merged: Option<bool>,
    pub upstream_state: String,
    /// Commits the upstream has that this branch does not, as of the last
    /// fetch. Absent when the branch has no upstream, the upstream is gone,
    /// or the count could not be read; `upstream_state` says which, so the
    /// Overview draws no cell, `?`, or a number rather than a silent zero.
    pub behind_upstream: Option<u32>,
    /// When a linked worktree was added: the creation time of its
    /// `.git/worktrees/<name>` entry. The main worktree has none, and it
    /// leads the Overview's list regardless.
    pub created_at_unix_ms: Option<u64>,
    pub unavailable_reason: Option<String>,
    pub last_fetch_at_unix_ms: Option<u64>,
    pub measured_at_unix_ms: Option<u64>,
    pub pane_count: usize,
    /// Who is working in this worktree and on what (PRD B34, B35, D-32,
    /// D-55). Overview's own value is width, so this is one line rather than
    /// a new area.
    pub agent_line: WorktreeAgentLineSnapshot,
    pub disk: DiskUsageSnapshot,
    pub pull_request: Option<PullRequestSnapshot>,
    pub github: GithubStatusSnapshot,
    pub deletion_gate: WorktreeDeletionGateSnapshot,
    pub open_error: Option<String>,
    pub path: String,
    pub branch: Option<String>,
    /// Git lists the worktree but its path is not on disk.
    pub missing: bool,
    pub is_main: bool,
    pub dirty: bool,
    pub changed_file_count: u32,
    /// What the ahead/behind and line counts are measured against: the pull
    /// request's base when there is one, the repository default branch
    /// otherwise, and absent when neither could be resolved.
    pub base_branch: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// Lines added and removed by commits on this branch since the base.
    /// Working-tree changes are deliberately excluded; the Changes view's
    /// uncommitted group is where those are counted.
    pub added_lines: u32,
    pub removed_lines: u32,
    pub unpushed: Option<UnpushedSnapshot>,
}

/// The Overview worktree row's agent line.
///
/// An empty `agents` with no reason is a worktree nobody is working in. An
/// empty one carrying a reason is a worktree Hide cannot see into, which is a
/// different answer and is drawn as one (PRD B35, D-60, `design/principles.md`
/// rule 9).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct WorktreeAgentLineSnapshot {
    /// The agents attached to this worktree's panes, in the sidebar's own
    /// order so the two screens name them the same way.
    pub agents: Vec<AgentChipSnapshot>,
    /// The same mark and sentence the pane header shows, when one of those
    /// agents is uninstrumented. The reason is the first in the resolution
    /// order among them, so the line agrees with the pane it came from.
    pub uninstrumented_reason: Option<String>,
    pub uninstrumented_label: Option<String>,
    pub uninstrumented_code: Option<String>,
}

/// One policy shared by all worktree deletion surfaces.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct WorktreeDeletionGateSnapshot {
    /// Why deletion cannot start: main, locked or unmeasured ignored folders.
    pub blocked_reason: Option<String>,
    /// What the operator should know before deleting, in the order shown.
    pub warnings: Vec<String>,
    pub button_label: String,
    /// The branch may go with the folder: it is named, is not the base, and
    /// the worktree is on disk.
    pub can_delete_branch: bool,
    /// What deleting the branch loses: commits the base does not have, or an
    /// answer Git could not give. Present means `git branch -D`.
    pub branch_warning: Option<String>,
    /// The checkbox that accepts losing what the folder holds (uncommitted
    /// files, a worktree inside it, or a status Git could not read). Present
    /// means deletion waits for it and then runs `git worktree remove --force`.
    pub discard_label: Option<String>,
}

/// The accepted deletion, measured before any pane closes and executed by
/// the worker only after Herdr confirms every pane is gone.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorktreeRemovalSnapshot {
    pub id: u64,
    /// The device the worktree is on; `None` is this machine. Its path and
    /// panes are that device's.
    pub device_id: Option<String>,
    pub repository_root: String,
    pub checkout_path: String,
    pub expected_head_sha: Option<String>,
    pub expected_branch: Option<String>,
    pub protected_base_branch: Option<String>,
    pub branch: Option<String>,
    pub delete_branch: bool,
    /// The branch goes with `git branch -D`: the operator was told it holds
    /// commits the base does not.
    pub force_delete_branch: bool,
    /// The operator accepted losing the folder's changes, so the recheck
    /// lets dirt through and Git removes with `--force`.
    pub discard_changes: bool,
    pub expected_ignored_repositories: Vec<String>,
    pub phase: String,
    pub message: Option<String>,
}

/// One user-initiated task whose blocking work runs outside the runtime mutex.
///
/// The shell observes this receipt to keep a sheet locked, focus a created
/// pane, or show the exact failed step. A single slot also makes a repeated
/// submission idempotent while an operation is in flight.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskOperationSnapshot {
    pub id: u64,
    /// The device the task runs on; `None` is this machine. Its paths and
    /// pane are that device's.
    pub device_id: Option<String>,
    pub kind: String,
    pub phase: String,
    pub repository_root: Option<String>,
    pub branch: Option<String>,
    pub base_branch: Option<String>,
    pub path: Option<String>,
    pub pane_id: Option<String>,
    pub agent_kind: Option<String>,
    pub message: Option<String>,
    /// How starting `agent_kind` in the created pane went, kept apart from
    /// the creation itself: `starting`, `started`, `failed`, or `unknown`
    /// when Herdr did not answer and the pane has to be looked at. `None`
    /// when the task started no agent.
    pub agent_phase: Option<String>,
    pub agent_message: Option<String>,
    /// The id the start surface sent with the request, so it follows its own
    /// task and never another window's or phone's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// The task came from a surface that is not the operator's own screen
    /// (a phone start): its tab is made without taking the screen, the
    /// keyboard or Herdr's focus. The core's own fact; no shell reads it.
    #[serde(skip)]
    pub stays_behind: bool,
}

/// The one repository clone Add a project started, and how far it got.
///
/// One slot, like `TaskOperationSnapshot`: a second clone while one runs is
/// refused, and the same URL and folder sent again joins the one running.
/// The URL is not here, because it can carry credentials; `host` is what the
/// shell shows. `phase` is `cloning`, `cancelling`, then `finished` (the
/// folder is registered through `create_workspace`), `failed` with `message`,
/// or `cancelled`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RepositoryCloneSnapshot {
    pub id: u64,
    pub host: String,
    /// The new folder's full path, `parent` and the name the URL gives.
    pub path: String,
    pub phase: String,
    /// Git's own stage, `Receiving objects` and the like, once it names one.
    pub stage: Option<String>,
    pub percent: Option<u8>,
    pub message: Option<String>,
}

/// The explorer's most recent filesystem change and how far it got.
///
/// One slot, like `TaskOperationSnapshot`: the shell reads the finished
/// phase to reload the parents of `path` and `destination`, and the failed
/// phase to draw `message` under the row the change started from. A new
/// request replaces a settled slot; a request while one is working is
/// refused, so a double-click cannot run the same rename twice.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExplorerOperationSnapshot {
    pub id: u64,
    pub kind: String,
    pub phase: String,
    /// The path the change started from: the item being renamed or moved,
    /// or the path a new item takes.
    pub path: String,
    /// The item's path once the change has landed; equal to `path` for a
    /// creation.
    pub destination: String,
    pub message: Option<String>,
}

/// One repository's worktrees.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ProjectWorktreesSnapshot {
    pub github: GithubStatusSnapshot,
    pub pull_requests: Vec<PullRequestSnapshot>,
    pub pull_request_window: String,
    pub shared_git_path: Option<String>,
    pub shared_git_disk: DiskUsageSnapshot,
    pub disk_total_bytes: Option<u64>,
    pub disk_confirmed_bytes: Option<u64>,
    pub linked_disk_bytes: Option<u64>,
    pub disk_unavailable_reason: Option<String>,
    pub base_branch: Option<String>,
    pub base_source: String,
    pub base_branch_fallback: Option<String>,
    pub root_path: String,
    pub default_branch: Option<String>,
    pub branches: Vec<String>,
    pub worktrees: Vec<WorktreeSnapshot>,
    /// Why this repository has no worktree list. An empty list with no reason
    /// means the repository genuinely has none.
    pub unavailable_reason: Option<String>,
    /// The GitHub repository its `origin` names (`[HOST/]OWNER/NAME`), by
    /// which a device's pull requests are read with this machine's login.
    #[serde(skip)]
    pub repository: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct WorktreeCatalogSnapshot {
    pub projects: Vec<ProjectWorktreesSnapshot>,
}

impl WorktreeCatalogSnapshot {
    pub fn project(&self, root_path: &str) -> Option<&ProjectWorktreesSnapshot> {
        self.projects
            .iter()
            .find(|project| project.root_path == root_path)
    }
}

/// How much disk one checkout occupies, and which of its top-level folders is
/// the biggest share of it.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct DiskUsageSnapshot {
    pub measured_at_unix_ms: Option<u64>,
    /// The checkout this measurement describes. A card whose selection has
    /// moved on compares this against its own path and shows `measuring`
    /// rather than the previous checkout's size.
    pub path: Option<String>,
    pub total_bytes: Option<u64>,
    pub largest_child_name: Option<String>,
    pub largest_child_bytes: Option<u64>,
    pub unavailable_reason: Option<String>,
    /// What a checkout holds by layer, for a measured checkout row. Absent
    /// for the shared Git directory and for a measurement that failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layers: Option<hide_node_link::disk::DiskLayers>,
    /// Free space of the volume the measured path sits on, in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_free_bytes: Option<u64>,
    /// The size of that volume.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_total_bytes: Option<u64>,
    /// The folders behind the layer cells. The core keeps them so a cleanup
    /// can move them; the wire never carries a path.
    #[serde(skip)]
    pub folders: Vec<hide_node_link::disk::LayerFolder>,
}

/// The right panel's summary card for the selected checkout.
///
/// The per-checkout git facts are not repeated here: they live on
/// [`CheckoutSnapshot`], which the card reads through the focused checkout.
/// What this carries is everything the card needs that a checkout row does
/// not: the repository's `gh` health, the one measured checkout's disk usage,
/// and whether the worktree may be removed.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct CheckoutCardSnapshot {
    /// Absent when nothing is selected, or when the selection is a remote
    /// checkout - remote worktree management is out of scope, so no card.
    pub checkout_id: Option<String>,
    pub github: GithubStatusSnapshot,
    pub disk: DiskUsageSnapshot,
    /// True while the selected checkout's size is still being measured.
    pub disk_measuring: bool,
    pub deletion_gate: Option<WorktreeDeletionGateSnapshot>,
    /// Current topology, not a claim about who authored HEAD or older commits.
    pub panes: Vec<CheckoutPaneContext>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CheckoutPaneContext {
    pub pane_id: String,
    pub title: String,
    pub status: String,
    pub session_id: Option<String>,
    pub parent_pane_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ImeSnapshot {
    pub marked_text: String,
    pub selected_range: TextRangeSnapshot,
    pub replacement_range: Option<TextRangeSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TextRangeSnapshot {
    pub location: u64,
    pub length: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StatusSnapshot {
    pub server_discovery: ServerDiscoverySnapshot,
    pub tab_rename: Option<TabRenameSnapshot>,
    pub herdr: ProviderStatusSnapshot,
    pub remote: Vec<RemoteStatusSnapshot>,
    pub environment: Vec<EnvironmentStatusSnapshot>,
    pub agent_hooks: AgentHooksSnapshot,
    pub background_ai: BackgroundAiSnapshot,
    /// The Host entries Add device offers (Settings › Devices).
    pub ssh_hosts: SshHostsSnapshot,
    pub diagnostics: Vec<DiagnosticSnapshot>,
    pub last_error: Option<LastErrorSnapshot>,
    /// Core-owned operations which are waiting for a transport result or an
    /// authoritative Herdr event. Keeping these beside status lets every
    /// surface show the same bounded, target-scoped state.
    pub async_operations: Vec<AsyncOperationSnapshot>,
    /// The core-owned outcome of the latest explicitly correlated pane-focus
    /// request. Ordinary focus events have no request id and do not replace
    /// this receipt, so a relationship control never mistakes another pane's
    /// error or an older focused layout for its own answer.
    pub pane_focus_request: Option<PaneFocusRequestSnapshot>,
    /// The answers to the latest `browser_open` requests that named
    /// themselves, oldest first and bounded, so a `hide browser open` waiting
    /// on one reads its own (issue 155).
    pub browser_opens: Vec<BrowserOpenReceiptSnapshot>,
}

/// The concrete Host entries of the account's `~/.ssh/config`, as Add device
/// lists them (PRD settings-cleanup D-19). Hide keeps only an alias and a name
/// for a device; this is read on request and never stored.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SshHostsSnapshot {
    /// `idle` before the first request, `loading` while one runs, `ready`
    /// once its answer is in; a later request shows the last answer as
    /// `loading` until the new one lands.
    pub state: String,
    pub hosts: Vec<SshHostSnapshot>,
    /// The config names more concrete aliases than are listed.
    pub truncated: bool,
}

impl Default for SshHostsSnapshot {
    fn default() -> Self {
        Self {
            state: "idle".to_owned(),
            hosts: Vec::new(),
            truncated: false,
        }
    }
}

/// One alias of the config.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SshHostSnapshot {
    pub alias: String,
    /// `user@host:port` as `ssh -G` resolved it; absent when it could not.
    pub address: Option<String>,
    /// The name of the registered device that already uses this alias or
    /// reaches the same address.
    pub added_as: Option<String>,
    /// Why there is no address: `ssh_missing`, `ssh_failed` or `timed_out`.
    pub problem: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ServerDiscoverySnapshot {
    pub loading: bool,
    pub failure: Option<String>,
}

/// One `browser_open` request's outcome: where the page opened, or why not.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BrowserOpenReceiptSnapshot {
    pub request_id: String,
    pub ok: bool,
    pub message: Option<String>,
    pub device_id: Option<String>,
    pub path: Option<String>,
    pub display_id: Option<String>,
}

/// One explicitly correlated pane-focus request and its core-owned outcome.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PaneFocusRequestSnapshot {
    pub request_id: String,
    pub target_pane_id: String,
    pub phase: String,
    pub message: Option<String>,
    pub retryable: bool,
}

/// Which agent and model Hide AI uses, and what each agent can do about it
/// right now (Settings › Hide AI).
///
/// The choice is the operator's, stored by `hide-ai` in its own file; the
/// availability and the model lists come from asking the providers, on the
/// coordinator's reader, never under the runtime mutex. Every reason is a
/// short machine code: the shell writes the words, and nothing here carries
/// an account, a path or a conversation (B68).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct BackgroundAiSnapshot {
    /// Use Hide AI (D-14). Off, no model is asked anything and the other
    /// settings are kept.
    pub enabled: bool,
    /// The agent Hide AI runs on first (Runs on), or `None` when nobody
    /// chose one and none can be chosen yet (B47): Hide features then work
    /// without a model, and the first agent that is switched on and signed in
    /// is chosen by itself (D-18, D-27).
    pub provider: Option<String>,
    /// Whether `provider` is a stored choice.
    pub chosen: bool,
    /// The `에이전트 요약` switch: agent labels are asked for and shown.
    pub agent_summary: bool,
    /// One row per agent Hide AI knows, in the fixed order. An agent that is
    /// not on this Mac is still a row, because "not here" and "not signed in"
    /// are different answers.
    pub providers: Vec<BackgroundAiProviderSnapshot>,
    /// The agents tried, in this order, when `provider` cannot answer; empty
    /// until the operator adds one (D-16).
    pub fallback: Vec<BackgroundAiFallbackSnapshot>,
    /// Why Runs on is not answering, and which listed agent answers instead;
    /// absent while the chosen agent answers (B41, B42).
    pub refusal: Option<BackgroundAiRefusalSnapshot>,
    /// Why the saved choice could not be read or written. The defaults are in
    /// use while this is set; it is never left empty to stand for success.
    pub unavailable_reason: Option<String>,
    /// The Usage limit: the share of an agent's week Hide AI may use before a
    /// background request moves to the next agent of the order. `None` is
    /// No limit (PRD hide-ai-usage-limit D-01).
    pub usage_limit: Option<u8>,
}

impl BackgroundAiSnapshot {
    /// Every provider, none of them asked yet. This is what the screen shows
    /// before the first read lands, and what an unobserved read answers. No
    /// choice is claimed, because nothing has been read that could say one.
    pub fn unread() -> Self {
        Self {
            enabled: true,
            providers: hide_ai::PROVIDERS
                .iter()
                .map(|provider| BackgroundAiProviderSnapshot::unread(*provider))
                .collect(),
            agent_summary: true,
            usage_limit: Some(hide_ai::DEFAULT_LIMIT_PERCENT),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct BackgroundAiProviderSnapshot {
    /// The provider layer's own id, such as `codex`.
    pub id: String,
    pub label: String,
    /// The id the Agents tab and the logos key the same agent by: the kit's
    /// adapter id (`claude-code`, `codex`, `grok`, `opencode`, `pi`,
    /// `cursor`), or `gemini-cli` for Gemini CLI, which Hide AI still uses
    /// though the kit no longer lists it.
    pub agent: String,
    /// The availability class the provider layer reported: `ready`,
    /// `needs_login`, `not_installed`, `unavailable`, `unsupported`, or
    /// `unread` before it has been asked.
    pub state: String,
    /// The short words beside the provider's name. The core writes them; no
    /// view builds a sentence out of `state`.
    pub headline: String,
    /// The provider layer's own reason code, when its state carries one
    /// (`cannot_guarantee_read_only`, `model_not_offered:<model>`).
    pub message: Option<String>,
    /// Whether the agent's program was found; false only for
    /// `not_installed`.
    pub installed: bool,
    /// Whether the agent's sign-in is checked without a request. False for
    /// Gemini CLI, whose `ready` only means its program was found, so the
    /// shell does not call it signed in and it is never chosen by itself.
    pub login_checked: bool,
    /// Whether Runs on and Add agent offer it: installed, signed in, a
    /// backend, and a call that cannot change files (D-15). Parked in a usage
    /// limit is still selectable.
    pub selectable: bool,
    /// When a usage limit ends, as unix milliseconds, when the agent said.
    pub retry_at_ms: Option<u64>,
    /// The model this agent is asked for; empty is the CLI's own default. For
    /// an agent in the fallback list it is that entry's model.
    pub model: String,
    /// The models this agent offers. Empty when they are not known, which
    /// `models_unavailable_reason` then says.
    pub models: Vec<String>,
    /// The list is the CLI's documented one and cannot be asked for (Gemini
    /// CLI), so the shell says so beside the menu (B36).
    pub models_fixed: bool,
    /// Whether "CLI default" can be chosen: the agent is asked with no
    /// `--model` (B36). True for the agents measured against nothing yet.
    pub cli_default: bool,
    pub models_unavailable_reason: Option<String>,
    /// The agent's weekly usage as the toolbar's usage read last answered it;
    /// absent when it gave no figure, and then the agent is never held back
    /// by the usage limit (PRD hide-ai-usage-limit D-07).
    pub weekly: Option<BackgroundAiWeekly>,
    /// The weekly figure is at or past the usage limit: this agent cannot
    /// answer a request Hide AI starts on its own (D-02).
    pub past_limit: bool,
}

/// One agent's weekly usage: the share spent and when the window resets.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct BackgroundAiWeekly {
    pub used_percent: f64,
    /// Unix milliseconds, when the agent said.
    pub resets_at_ms: Option<u64>,
}

impl BackgroundAiProviderSnapshot {
    pub fn unread(provider: hide_ai::ProviderId) -> Self {
        Self {
            id: provider.as_str().to_owned(),
            label: provider.label().to_owned(),
            agent: provider.descriptor().agent.to_owned(),
            state: "unread".to_owned(),
            headline: "Not checked yet".to_owned(),
            installed: true,
            login_checked: provider.login_probe(),
            model: provider.default_model().to_owned(),
            cli_default: provider.default_model().is_empty(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BackgroundAiFallbackSnapshot {
    pub provider: String,
    /// The model it is asked for; empty is the CLI's default.
    pub model: String,
}

/// "Using Codex · Claude Code is out of usage until 3:10 PM" (B41), or the
/// Runs on row's reason when nothing answers (B42). Codes only.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BackgroundAiRefusalSnapshot {
    /// The agent that cannot answer: Runs on.
    pub provider: String,
    /// The router's availability class for it: `usage_limited`,
    /// `past_limit` (at or past the operator's usage limit), `needs_login`,
    /// `not_installed`, `unavailable` or `unsupported`.
    pub reason: String,
    /// When a usage limit ends, as unix milliseconds, when the agent said.
    pub retry_at_ms: Option<u64>,
    /// The listed agent answering instead; absent when none can (B42).
    pub using: Option<String>,
    /// For `past_limit`: the agent's weekly share and the limit it is past,
    /// so the screen says `past your 90% limit` from the core's own numbers.
    pub used_percent: Option<f64>,
    pub limit: Option<u8>,
}

/// What the Settings diagnosis says about agent hooks (PRD B27, B28, D-31,
/// D-48).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct AgentHooksSnapshot {
    /// Each machine's hook parts are its kit rows (`DeviceSnapshot.kit`,
    /// PRD device-parity B27), and the sessions Hide does not hear are
    /// counted on those rows' `sessions` and named on each pane's
    /// `children.connection` (PRD settings-cleanup D-23: one per-session
    /// warning list that grew with every session is gone).
    /// The sentence describing the last hook report Herdr did not take, when
    /// the most recent report failed. It is what separates "installed but
    /// every report is refused" from the restart advice above: with it on
    /// screen, a restart is not the fix and the sentence says what is.
    pub last_report_failure: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EnvironmentStatusSnapshot {
    pub key: String,
    pub required: bool,
    pub format: String,
    pub state: String,
    pub absent_behavior: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DiagnosticSnapshot {
    pub kind: String,
    pub message: String,
    pub occurred_at: u64,
}

/// Carries no "last checked" stamp on purpose. Every check restamped it, the
/// stamp rides `rest`, and `rest` is compared field by field to decide whether
/// the reader is current - so a clock reading nothing renders made every
/// heartbeat re-send the whole navigator, ui state, status and pet.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProviderStatusSnapshot {
    pub state: String,
    pub socket_path: Option<String>,
    pub message: Option<String>,
    pub expected_protocol: Option<u64>,
    pub received_protocol: Option<u64>,
    pub received_version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RemoteStatusSnapshot {
    pub target_id: String,
    pub state: String,
    pub message: Option<String>,
    /// The version reported by `herdr status server --json` on this device.
    /// A missing version keeps version-gated mutations disabled.
    pub herdr_version: Option<String>,
    pub session: Option<RemoteSessionSnapshot>,
    pub files: RemoteFileListSnapshot,
    /// Whether the session's projects are grouped from the device's own
    /// facts (`device_catalog`).
    pub catalog: DeviceCatalogSnapshot,
    /// Since when the device has not answered (PRD overview-device-tasks
    /// B4): set when its row stops being `connected`, or when it first
    /// waits for its connection, and cleared when it answers; absent from
    /// the wire while it is connected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unreachable_since_unix_ms: Option<u64>,
}

/// How far a device's projects are confirmed by its helper (PRD S5.5 B3,
/// B4). `resolving`: the helper is being asked; `ready`: every directory the
/// device's Herdr names has the helper's answer; `unavailable`: the helper
/// cannot be asked now and `message` says why. A directory the helper has not
/// answered for is shown as its Herdr workspace alone, never grouped by a
/// guess and never read on this machine.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DeviceCatalogSnapshot {
    pub state: String,
    pub message: Option<String>,
    /// Directories the helper answered with a refusal (missing, unreadable),
    /// each with its reason; their workspaces stay ungrouped.
    pub refused: Vec<DeviceCatalogRefusal>,
}

impl Default for DeviceCatalogSnapshot {
    fn default() -> Self {
        Self {
            state: "resolving".to_owned(),
            message: None,
            refused: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DeviceCatalogRefusal {
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RemoteFileListSnapshot {
    pub root_path: Option<String>,
    pub state: String,
    pub entries: Vec<RemoteFileEntrySnapshot>,
    pub message: Option<String>,
    pub generation: u64,
}

impl RemoteFileListSnapshot {
    pub fn idle() -> Self {
        Self {
            root_path: None,
            state: "idle".to_owned(),
            entries: Vec::new(),
            message: None,
            generation: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RemoteFileEntrySnapshot {
    pub path: String,
    pub name: String,
    pub is_directory: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RemoteSessionSnapshot {
    pub workspaces: Vec<WorkspaceSnapshot>,
    pub agents: Vec<SidebarAgentSnapshot>,
    pub active_tab_ids: BTreeMap<String, String>,
    pub focused_workspace_id: Option<String>,
    pub focused_checkout_id: Option<String>,
    pub focused_tab_id: Option<String>,
    pub focused_pane_id: Option<String>,
    pub pane_layouts: Vec<RemotePaneLayoutSnapshot>,
    /// Each device pane's Hide hook tokens, by its remote pane id. The core
    /// judges a device pane's children from them and the device's kit
    /// (`Runtime::derive_device_session`), so they are not sent to the shell.
    #[serde(skip)]
    pub pane_hook_tokens: BTreeMap<String, crate::agent_hooks::PaneHookTokens>,
    /// The issue tokens of the device's Herdr: a workspace's by its Herdr
    /// id, a pane's by its remote pane id. The core reads a node checkout's
    /// issue link from them (`Runtime::project_node_issues`), so they are not
    /// sent to the shell either.
    #[serde(skip)]
    pub issue_tokens: crate::wire::IssueTokens,
    /// The GitHub issues a node project's checkouts link to, by project
    /// root, made with its rows (`Runtime::project_node_issues`) for the
    /// node's GitHub read to bring the ones its open list leaves out; empty
    /// in a raw session. Not sent to the shell.
    #[serde(skip)]
    pub issue_links: BTreeMap<String, Vec<crate::issues::IssueReference>>,
    /// The version the device's Herdr server named in the snapshot this
    /// session was read from, which follows a live handoff where the status
    /// read the connection made once does not (PRD device-herdr-update B3).
    /// Not sent to the shell: the device's remote status carries it.
    #[serde(skip)]
    pub herdr_version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RemotePaneLayoutSnapshot {
    pub workspace_id: String,
    pub tab_id: String,
    pub focused_pane_id: String,
    pub zoomed: bool,
    pub frames: Vec<RemotePaneLayoutFrame>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RemotePaneLayoutFrame {
    pub pane_id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LastErrorSnapshot {
    pub kind: String,
    pub message: String,
    pub retryable: bool,
    pub occurred_at: u64,
    /// The request this refusal answers, when the event carried one, so the
    /// surface that sent it (the start panel, a phone) reads its own answer
    /// and no other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl Snapshot {
    pub fn initial(options: &CoreOptions) -> Self {
        let herdr_state = if options.herdr_socket_path.is_some() {
            "not_connected"
        } else {
            "unconfigured"
        };
        let herdr_message = if options.herdr_socket_path.is_some() {
            Some("Waiting for the first herdr connection attempt".to_owned())
        } else {
            Some("No herdr socket path was configured".to_owned())
        };

        Self {
            schema_version: SCHEMA_VERSION,
            usage: crate::usage::model::UsageSnapshot::initial(options.node_id.as_str()),
            delivery_watches: Vec::new(),
            navigator: NavigatorSnapshot {
                agent_scope: Default::default(),
                root_path: None,
                changes_root_path: None,
                focused_device_id: None,
                focused_workspace_id: None,
                focused_checkout_id: None,
                devices: vec![crate::workspace::local_device(
                    &options.node_id,
                    options.machine_name.as_deref(),
                    None,
                )],
                workspaces: Vec::new(),
                inactive_projects: Vec::new(),
                no_agent_projects: Vec::new(),
                agents: Vec::new(),
                hide_ai_checkouts: Vec::new(),
                sleeping_sessions: Vec::new(),
                archived_sessions: Vec::new(),
                factory_worker_panes: Vec::new(),
            },
            overlay: OverlaySnapshot {
                kind: None,
                title: None,
                message: None,
                actions: Vec::new(),
            },
            tab: TabSnapshot {
                agent: None,
                naming: Default::default(),
                id: None,
                workspace_id: None,
                checkout_id: None,
                label: None,
                empty: true,
                delegated: false,
                panes: Vec::new(),
            },
            connection: ConnectionSnapshot {
                kind: "local".to_owned(),
                state: "not_connected".to_owned(),
                target_id: None,
            },
            zoomed: None,
            focused: FocusedSnapshot {
                surface: Surface::Terminal,
                pane_id: None,
                operator_focus: Vec::new(),
            },
            pane_layouts: Vec::new(),
            terminal: TerminalSnapshot {
                headers: BTreeMap::new(),
                pane_id: None,
                closed: false,
                exit_code: None,
                panes: Vec::new(),
                input_requests: Vec::new(),
            },
            editor: EditorSnapshot {
                tabs: Vec::new(),
                active_tab_id: None,
                document: None,
                archive_detail: None,
                opening: Vec::new(),
            },
            sessions: SessionsSnapshot::default(),
            changes: Edited::default(),
            card: CheckoutCardSnapshot::default(),
            git_worktrees: None,
            git_worktrees_loading: true,
            git_worktrees_remote: false,
            worktree_removal: None,
            task_operation: None,
            repository_clone: None,
            explorer_operation: None,
            issue_work: IssueWorkSnapshot::default(),
            pr_work: PrWorkSnapshot::default(),
            find: PaneFindSnapshot::default(),
            ui_state: UiStateSnapshot::default(),
            ime: ImeSnapshot {
                marked_text: String::new(),
                selected_range: TextRangeSnapshot {
                    location: 0,
                    length: 0,
                },
                replacement_range: None,
            },
            status: StatusSnapshot {
                tab_rename: None,
                herdr: ProviderStatusSnapshot {
                    state: herdr_state.to_owned(),
                    socket_path: options.herdr_socket_path.clone(),
                    message: herdr_message,
                    expected_protocol: None,
                    received_protocol: None,
                    received_version: None,
                },
                remote: Vec::new(),
                environment: Vec::new(),
                agent_hooks: AgentHooksSnapshot::default(),
                background_ai: BackgroundAiSnapshot::unread(),
                ssh_hosts: SshHostsSnapshot::default(),
                diagnostics: Vec::new(),
                last_error: None,
                async_operations: Vec::new(),
                pane_focus_request: None,
                browser_opens: Vec::new(),
                server_discovery: ServerDiscoverySnapshot {
                    loading: true,
                    failure: None,
                },
            },
            pet: PetSnapshot::initial(),
            recent_closed: RecentClosedSnapshot::default(),
            workspace_view: None,
            browser_views: Vec::new(),
            browser_scopes: Vec::new(),
            browser_views_revision: None,
            memory: None,
            link_summaries: None,
            link_panel: None,
            session_record: None,
            session_work_search: None,
            factory: None,
            factory_task: None,
        }
    }
}

impl PetSnapshot {
    pub fn initial() -> Self {
        Self {
            visible: true,
            connection: "not_connected".to_owned(),
            connection_message: Some("Waiting for the first herdr connection attempt".to_owned()),
            pose: "disconnected".to_owned(),
            sleep_phase: "awake".to_owned(),
            roam_allowed: false,
            badges: PetBadgesSnapshot::default(),
            attention_pane_ids: Vec::new(),
            origin: None,
            shortcut: None,
            shortcut_error: None,
            theme_id: "default".to_owned(),
        }
    }
}

/// The sections of [`Snapshot`] that ride the revisioned `rest` channel of
/// the delta wire: everything except the editor and the changes view (each
/// with its own revision), the terminal chunk ring (sequence cursor), and the
/// per-event scalars, which now include find state.
/// Owned copy retained by the runtime to stamp revisions by comparison, so
/// no mutation site needs dirty-tracking discipline.
mod rest;
pub use rest::{RestSections, RestWire};

/// One delta response taken from the runtime, holding everything the wire
/// needs and nothing that reaches back into the runtime.
///
/// Taking a payload is what runs under the runtime lock; serializing it is
/// what must not. Sections ride the reference-counted copies the runtime
/// already retains for revision stamping, so taking one copies no section body.
pub struct SnapshotDeltaPayload {
    /// Cursor normalized to zero on reset, for negotiated semantic deltas.
    pub have_revision: u64,
    /// Complete immutable state for typed internal projections.
    pub current_rest: Arc<RestSections>,
    pub schema_version: u32,
    pub revision: u64,
    pub rest: Option<Arc<RestSections>>,
    pub usage: Option<Arc<crate::usage::model::UsageSnapshot>>,
    pub editor: Option<Arc<EditorSnapshot>>,
    pub changes: Option<Arc<ChangesSnapshot>>,
    /// The visible View documents (PRD S7 contract 3.1); always `None` in a
    /// shell without View areas.
    pub documents: Option<DocumentsDelta>,
    pub memory: Option<Arc<MemorySnapshot>>,
    pub link_summaries: Option<Arc<crate::links::LinkSummariesSnapshot>>,
    pub link_panel: Option<Arc<crate::links::LinkPanelSnapshot>>,
    pub session_record: Option<Arc<crate::links::ledger::SessionRecordSnapshot>>,
    pub session_work_search: Option<Arc<crate::links::ledger::SessionSearchSnapshot>>,
    pub factory: Option<crate::factory::screen::FactorySection>,
    /// `Some(None)`: the open Task page closed since the reader's revision.
    pub factory_task: Option<Option<crate::factory::screen::FactoryTaskSection>>,
    pub find: PaneFindSnapshot,
}

/// The documents the front Workspace's area-active displays show, in tree
/// order, and those among them that changed past the reader's revision. The
/// shell keeps the documents it has whose ids are in `visible`, overwrites
/// `changed`, and drops the rest, so a keystroke re-sends one document.
pub struct DocumentsDelta {
    pub visible: Vec<String>,
    pub changed: Vec<(String, Arc<EditorDocumentSnapshot>)>,
}

/// One delta response on the snapshot wire. `rest`, `editor`, and `changes`
/// are present only when the caller's `have_revision` predates their last
/// change; terminal output travels in frames of its own (PRD
/// core-host-node-terminal D-15), never here. The changes view holds a whole file's diff text, so it is kept off `rest`,
/// which restamps whenever any agent row changes.
///
/// It borrows from a `SnapshotDeltaPayload`, never from the runtime, so
/// building and serializing it needs no lock.
#[derive(Serialize)]
pub struct SnapshotDeltaWire<'a> {
    pub schema_version: u32,
    pub revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rest_format: Option<&'static str>,
    pub rest: Option<RestWire<'a>>,
    /// Usage-only changes never resend the navigator or unrelated UI state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<&'a crate::usage::model::UsageSnapshot>,
    pub editor: Option<&'a EditorSnapshot>,
    pub changes: Option<&'a ChangesSnapshot>,
    /// Absent unless a visible View document changed for this reader, and
    /// always absent in a shell without View areas, so such a wire keeps
    /// exactly its keys.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documents: Option<DocumentsWire<'a>>,
    /// Omitted until Memory has a Project open or a live session given a
    /// card.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<&'a MemorySnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_summaries: Option<&'a crate::links::LinkSummariesSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_panel: Option<&'a crate::links::LinkPanelSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_record: Option<&'a crate::links::ledger::SessionRecordSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_work_search: Option<&'a crate::links::ledger::SessionSearchSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factory: Option<&'a crate::factory::screen::FactorySection>,
    /// `null` when the page closed since the reader's revision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factory_task: Option<Option<&'a crate::factory::screen::FactoryTaskSection>>,
    /// Find state rides top-level rather than in `rest`, because it changes on
    /// every keystroke while a search is open. In `rest` each keystroke would
    /// restamp that revision and resend the whole navigator, ui state, and pet
    /// sections with it - the wire would be sized by total state instead of by
    /// what changed. Six scalars on every response cost far less.
    pub find: &'a PaneFindSnapshot,
}

impl<'a> SnapshotDeltaWire<'a> {
    /// Only clients which negotiated this representation receive nested omissions.
    pub fn granular(payload: &'a SnapshotDeltaPayload) -> Self {
        let mut wire = Self::borrow(payload);
        wire.rest_format = Some("semantic-rest-v1");
        wire.rest = payload.rest.as_deref().map(|rest| RestWire {
            rest,
            have_revision: payload.have_revision,
            legacy: false,
        });
        wire
    }

    pub fn borrow(payload: &'a SnapshotDeltaPayload) -> Self {
        Self {
            schema_version: payload.schema_version,
            revision: payload.revision,
            rest_format: None,
            rest: payload.rest.as_deref().map(|rest| RestWire {
                rest,
                have_revision: 0,
                legacy: true,
            }),
            usage: payload.usage.as_deref(),
            editor: payload.editor.as_deref(),
            changes: payload.changes.as_deref(),
            documents: payload.documents.as_ref().map(|documents| DocumentsWire {
                visible: &documents.visible,
                changed: documents
                    .changed
                    .iter()
                    .map(|(tab_id, document)| ChangedDocumentWire { tab_id, document })
                    .collect(),
            }),
            memory: payload.memory.as_deref(),
            link_summaries: payload.link_summaries.as_deref(),
            link_panel: payload.link_panel.as_deref(),
            session_record: payload.session_record.as_deref(),
            session_work_search: payload.session_work_search.as_deref(),
            factory: payload.factory.as_ref(),
            factory_task: payload.factory_task.as_ref().map(Option::as_ref),
            find: &payload.find,
        }
    }
}

#[derive(Serialize)]
pub struct DocumentsWire<'a> {
    pub visible: &'a [String],
    pub changed: Vec<ChangedDocumentWire<'a>>,
}

#[derive(Serialize)]
pub struct ChangedDocumentWire<'a> {
    pub tab_id: &'a str,
    pub document: &'a EditorDocumentSnapshot,
}

#[cfg(test)]
mod interface_language_tests {
    use super::InterfaceLanguage;

    /// The cases `web/src/i18n/locale.test.ts` resolves, so a closed-window
    /// reader in the core and a shell agree on the system's language.
    #[test]
    fn a_system_language_resolves_as_the_shell_resolves_it() {
        for (tag, expected) in [
            ("en-GB", Some(InterfaceLanguage::English)),
            ("ko-KR", Some(InterfaceLanguage::Korean)),
            ("ja-JP", Some(InterfaceLanguage::Japanese)),
            ("zh-CN", Some(InterfaceLanguage::SimplifiedChinese)),
            ("zh-Hans", Some(InterfaceLanguage::SimplifiedChinese)),
            ("zh-Hans-KR", Some(InterfaceLanguage::SimplifiedChinese)),
            ("zh-SG", Some(InterfaceLanguage::SimplifiedChinese)),
            ("zh", Some(InterfaceLanguage::SimplifiedChinese)),
            ("zh-TW", None),
            ("zh-Hant", None),
            ("zh-Hant-HK", None),
            ("fr-FR", None),
            ("", None),
        ] {
            assert_eq!(InterfaceLanguage::from_system(tag), expected, "{tag}");
        }
    }
}

#[cfg(test)]
mod memory_start_failure_tests {
    use super::{MemoryStartFailure, MemoryStartFailureCode};

    /// The failed card shows why its agent never started: the question its
    /// shell stopped at, and never more than that one line.
    #[test]
    fn a_start_failure_carries_its_code_and_at_most_the_question_line() {
        let question = crate::coordination::refusal(
            crate::agent_start::SHELL_STARTING,
            "[oh-my-zsh] Would you like to update? [Y/n]",
        );
        assert_eq!(
            MemoryStartFailure::of(&question),
            MemoryStartFailure {
                code: MemoryStartFailureCode::ShellStarting,
                detail: Some("[oh-my-zsh] Would you like to update? [Y/n]".to_owned()),
            }
        );
        let long =
            crate::coordination::refusal(crate::agent_start::SHELL_STARTING, &"y".repeat(400));
        assert_eq!(
            MemoryStartFailure::of(&long)
                .detail
                .map(|line| line.chars().count()),
            Some(MemoryStartFailure::DETAIL_CHARS)
        );
        assert_eq!(
            MemoryStartFailure::of(crate::agent_start::SHELL_STARTING).detail,
            None,
            "a screen that could not be read"
        );
        assert_eq!(
            MemoryStartFailure::of("agent_not_started"),
            MemoryStartFailure {
                code: MemoryStartFailureCode::AgentNotStarted,
                detail: None,
            }
        );
        assert_eq!(
            MemoryStartFailure::of("agent_not_installed"),
            MemoryStartFailure {
                code: MemoryStartFailureCode::Other,
                detail: None,
            }
        );
        assert_eq!(
            MemoryStartFailure::of("card_not_processed\u{1f}The owning guide is unavailable"),
            MemoryStartFailure {
                code: MemoryStartFailureCode::CardNotProcessed,
                detail: Some("The owning guide is unavailable".to_owned()),
            }
        );
    }
}

#[cfg(test)]
mod pull_request_state_tests {
    use super::PullRequestSnapshot;

    /// A checkout's pull request reaches the shell with the one state every
    /// PR mark draws, so no shell derives it from the badge, draft flag,
    /// checks and review again (docs/status-model.md, Pull request visual
    /// states); the core-only facts stay off the wire.
    #[test]
    fn a_checkout_pull_request_carries_its_one_state_on_the_wire() {
        for (badge, draft, checks, review, expected) in [
            ("open", false, "failed", None, "failed"),
            (
                "review",
                false,
                "passing",
                Some("changes_requested"),
                "failed",
            ),
            (
                "review",
                false,
                "passing",
                Some("review_required"),
                "pending",
            ),
            ("open", false, "unknown", None, "pending"),
            ("review", false, "passing", Some("approved"), "mergeable"),
            ("open", true, "failed", None, "draft"),
            ("merged", true, "failed", None, "merged"),
            ("closed", false, "passing", None, "closed"),
        ] {
            let pull: PullRequestSnapshot = serde_json::from_value(serde_json::json!({
                "closing_issues": [], "title": "t", "checks": checks, "number": 7,
                "head_branch": "feature", "base_branch": "main", "url": "https://github.com/acme/app/pull/7",
                "badge": badge, "review": review, "is_draft": draft,
                "merged_at_unix_ms": null, "updated_at_unix_ms": null,
                "head_oid": "abc", "cross_repository": true,
            }))
            .expect("pull request decodes");
            let wire = serde_json::to_value(&pull).expect("pull request encodes");
            assert_eq!(
                wire["state"], expected,
                "{badge} {draft} {checks} {review:?}"
            );
            assert!(wire.get("head_oid").is_none() && wire.get("cross_repository").is_none());
        }
    }
}

#[cfg(test)]
mod wire_enum_tests {
    //! The shell decodes these strings strictly, so the values the core emits
    //! are a contract, pinned in `contracts/snapshot-wire-enums.json` and
    //! decoded by the shell's `SnapshotWireEnumTests`. Each list below is
    //! matched exhaustively: a new variant fails to compile here until it is
    //! listed, and then fails this test until it is in the contract file.
    use super::*;

    fn contract() -> serde_json::Map<String, serde_json::Value> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../contracts/snapshot-wire-enums.json"
        );
        let text = std::fs::read_to_string(path).expect("contracts/snapshot-wire-enums.json");
        match serde_json::from_str(&text).expect("contract is JSON") {
            serde_json::Value::Object(map) => map,
            other => panic!("contract must be an object, got {other}"),
        }
    }

    fn assert_wire<T: Serialize>(
        contract: &serde_json::Map<String, serde_json::Value>,
        key: &str,
        variants: &[T],
    ) {
        let listed = contract[key]
            .as_array()
            .unwrap_or_else(|| panic!("{key} must be an array"))
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .unwrap_or_else(|| panic!("{key} holds strings"))
                    .to_owned()
            })
            .collect::<Vec<_>>();
        let emitted = variants
            .iter()
            .map(|variant| match serde_json::to_value(variant).unwrap() {
                serde_json::Value::String(value) => value,
                other => panic!("{key} serializes as a string, got {other}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            emitted, listed,
            "{key}: the core emits the left, the contract lists the right"
        );
    }

    #[test]
    fn every_snapshot_enum_value_is_in_the_contract() {
        let contract = contract();
        let mut checked = BTreeSet::new();

        let origins = [
            CheckoutPurposeOrigin::Token,
            CheckoutPurposeOrigin::BranchDescription,
            CheckoutPurposeOrigin::AgentTitle,
            CheckoutPurposeOrigin::PullRequestTitle,
        ];
        for variant in origins {
            match variant {
                CheckoutPurposeOrigin::Token
                | CheckoutPurposeOrigin::BranchDescription
                | CheckoutPurposeOrigin::AgentTitle
                | CheckoutPurposeOrigin::PullRequestTitle => {}
            }
        }
        assert_wire(&contract, "checkout_purpose_origin", &origins);
        checked.insert("checkout_purpose_origin");

        let badges = [
            PullRequestBadge::Merged,
            PullRequestBadge::Closed,
            PullRequestBadge::Review,
            PullRequestBadge::Open,
        ];
        for variant in badges {
            match variant {
                PullRequestBadge::Merged
                | PullRequestBadge::Closed
                | PullRequestBadge::Review
                | PullRequestBadge::Open => {}
            }
        }
        assert_wire(&contract, "pull_request_badge", &badges);
        checked.insert("pull_request_badge");

        let decisions = [
            ReviewDecision::ReviewRequired,
            ReviewDecision::ChangesRequested,
            ReviewDecision::Approved,
        ];
        for variant in decisions {
            match variant {
                ReviewDecision::ReviewRequired
                | ReviewDecision::ChangesRequested
                | ReviewDecision::Approved => {}
            }
        }
        assert_wire(&contract, "review_decision", &decisions);
        checked.insert("review_decision");

        let checks = [
            PullRequestChecks::Unknown,
            PullRequestChecks::None,
            PullRequestChecks::Pending,
            PullRequestChecks::Failed,
            PullRequestChecks::Passing,
        ];
        for variant in checks {
            match variant {
                PullRequestChecks::Unknown
                | PullRequestChecks::None
                | PullRequestChecks::Pending
                | PullRequestChecks::Failed
                | PullRequestChecks::Passing => {}
            }
        }
        assert_wire(&contract, "pull_request_checks", &checks);
        checked.insert("pull_request_checks");

        use crate::agent_state::RequestVerb;
        let verbs = [
            RequestVerb::Answer,
            RequestVerb::Blocked,
            RequestVerb::Fix,
            RequestVerb::Review,
            RequestVerb::Stopped,
            RequestVerb::Result,
            RequestVerb::Working,
            RequestVerb::Waiting,
            RequestVerb::Idle,
        ];
        for variant in verbs {
            match variant {
                RequestVerb::Answer
                | RequestVerb::Blocked
                | RequestVerb::Fix
                | RequestVerb::Review
                | RequestVerb::Stopped
                | RequestVerb::Result
                | RequestVerb::Working
                | RequestVerb::Waiting
                | RequestVerb::Idle => {}
            }
        }
        assert_wire(&contract, "request_verb", &verbs);
        checked.insert("request_verb");

        let waits = [
            AgentWait::Children,
            AgentWait::Background,
            AgentWait::Reply,
            AgentWait::Checks,
        ];
        for variant in waits {
            match variant {
                AgentWait::Children
                | AgentWait::Background
                | AgentWait::Reply
                | AgentWait::Checks => {}
            }
        }
        assert_wire(&contract, "agent_wait", &waits);
        checked.insert("agent_wait");

        use crate::agent_state::RowMark;
        let marks = [
            RowMark::Turn,
            RowMark::Going,
            RowMark::Waiting,
            RowMark::Done,
            RowMark::Unread,
            RowMark::Quiet,
            RowMark::Unknown,
            RowMark::Asleep,
        ];
        for variant in marks {
            match variant {
                RowMark::Turn
                | RowMark::Going
                | RowMark::Waiting
                | RowMark::Done
                | RowMark::Unread
                | RowMark::Quiet
                | RowMark::Unknown
                | RowMark::Asleep => {}
            }
        }
        assert_wire(&contract, "row_mark", &marks);
        checked.insert("row_mark");

        let codes = [
            AgentStatusCode::Error,
            AgentStatusCode::Question,
            AgentStatusCode::Approval,
            AgentStatusCode::Working,
            AgentStatusCode::Done,
            AgentStatusCode::Idle,
            AgentStatusCode::Unknown,
            AgentStatusCode::Waiting,
            AgentStatusCode::Unfinished,
            AgentStatusCode::Stopped,
            AgentStatusCode::Attached,
            AgentStatusCode::Sleeping,
            AgentStatusCode::Waking,
            AgentStatusCode::SleepFailed,
        ];
        for variant in codes {
            match variant {
                AgentStatusCode::Error
                | AgentStatusCode::Question
                | AgentStatusCode::Approval
                | AgentStatusCode::Working
                | AgentStatusCode::Done
                | AgentStatusCode::Idle
                | AgentStatusCode::Unknown
                | AgentStatusCode::Waiting
                | AgentStatusCode::Unfinished
                | AgentStatusCode::Stopped
                | AgentStatusCode::Attached
                | AgentStatusCode::Sleeping
                | AgentStatusCode::Waking
                | AgentStatusCode::SleepFailed => {}
            }
        }
        assert_wire(&contract, "agent_status_code", &codes);
        checked.insert("agent_status_code");

        use crate::labels::analysis::LabelEnd;
        let ends = [
            LabelEnd::Working,
            LabelEnd::Question,
            LabelEnd::Blocked,
            LabelEnd::Done,
            LabelEnd::Waiting,
            LabelEnd::Unfinished,
        ];
        for variant in ends {
            match variant {
                LabelEnd::Working
                | LabelEnd::Question
                | LabelEnd::Blocked
                | LabelEnd::Done
                | LabelEnd::Waiting
                | LabelEnd::Unfinished => {}
            }
        }
        assert_wire(&contract, "label_end", &ends);
        checked.insert("label_end");

        use crate::links::ledger::ChildWork;
        let children = [
            ChildWork::Done,
            ChildWork::Working,
            ChildWork::Other,
            ChildWork::Unjudged,
        ];
        for variant in children {
            match variant {
                ChildWork::Done | ChildWork::Working | ChildWork::Other | ChildWork::Unjudged => {}
            }
        }
        assert_wire(&contract, "child_work", &children);
        checked.insert("child_work");

        use hide_session::turns::UserTurnKind;
        let user_turns = [UserTurnKind::PlanApproval, UserTurnKind::Question];
        for variant in user_turns {
            match variant {
                UserTurnKind::PlanApproval | UserTurnKind::Question => {}
            }
        }
        assert_wire(&contract, "user_turn_kind", &user_turns);
        checked.insert("user_turn_kind");

        let statuses = [
            ChangedFileStatus::Modified,
            ChangedFileStatus::Added,
            ChangedFileStatus::Deleted,
            ChangedFileStatus::Untracked,
            ChangedFileStatus::Renamed,
            ChangedFileStatus::Conflict,
        ];
        for variant in statuses {
            match variant {
                ChangedFileStatus::Modified
                | ChangedFileStatus::Added
                | ChangedFileStatus::Deleted
                | ChangedFileStatus::Untracked
                | ChangedFileStatus::Renamed
                | ChangedFileStatus::Conflict => {}
            }
        }
        assert_wire(&contract, "changed_file_status", &statuses);
        checked.insert("changed_file_status");

        let tab_kinds = [
            EditorTabKind::File,
            EditorTabKind::Diff,
            EditorTabKind::Session,
        ];
        for variant in tab_kinds {
            match variant {
                EditorTabKind::File | EditorTabKind::Diff | EditorTabKind::Session => {}
            }
        }
        assert_wire(&contract, "editor_tab_kind", &tab_kinds);
        checked.insert("editor_tab_kind");

        let document_kinds = [
            DocumentKind::Text,
            DocumentKind::Markdown,
            DocumentKind::Image,
            DocumentKind::Pdf,
            DocumentKind::Binary,
        ];
        for variant in document_kinds {
            match variant {
                DocumentKind::Text
                | DocumentKind::Markdown
                | DocumentKind::Image
                | DocumentKind::Pdf
                | DocumentKind::Binary => {}
            }
        }
        assert_wire(&contract, "document_kind", &document_kinds);
        checked.insert("document_kind");

        let sections = [
            RightPanelSection::Overview,
            RightPanelSection::Explorer,
            RightPanelSection::Changes,
            RightPanelSection::AgentSessions,
            RightPanelSection::Sessions,
            RightPanelSection::Memory,
        ];
        for variant in sections {
            match variant {
                RightPanelSection::Overview
                | RightPanelSection::Explorer
                | RightPanelSection::Changes
                | RightPanelSection::AgentSessions
                | RightPanelSection::Sessions
                | RightPanelSection::Memory => {}
            }
        }
        assert_wire(&contract, "right_panel_section", &sections);
        checked.insert("right_panel_section");

        use crate::agent_state::escalation::{MarkKind, Verb};
        use crate::agent_state::sessions::Group;
        let session_groups = crate::agent_state::sessions::GROUPS;
        for group in session_groups {
            match group {
                Group::NeedsYou | Group::Working | Group::Done | Group::Idle | Group::Resolved => {}
            }
        }
        assert_wire(&contract, "session_group", &session_groups);
        checked.insert("session_group");
        let lenses = [SidebarLens::Projects, SidebarLens::Tasks];
        for variant in lenses {
            match variant {
                SidebarLens::Projects | SidebarLens::Tasks => {}
            }
        }
        assert_wire(&contract, "sidebar_lens", &lenses);
        checked.insert("sidebar_lens");

        let views = [TaskView::Status, TaskView::Project];
        for variant in views {
            match variant {
                TaskView::Status | TaskView::Project => {}
            }
        }
        assert_wire(&contract, "task_view", &views);
        checked.insert("task_view");
        let verbs = [Verb::Approval, Verb::Answer, Verb::Confirm, Verb::Draft];
        for verb in verbs {
            match verb {
                Verb::Approval | Verb::Answer | Verb::Confirm | Verb::Draft => {}
            }
        }
        assert_wire(&contract, "raise_verb", &verbs);
        checked.insert("raise_verb");
        let pr_states = [
            PrState::Failed,
            PrState::Pending,
            PrState::Mergeable,
            PrState::Draft,
            PrState::Merged,
            PrState::Closed,
        ];
        for state in pr_states {
            match state {
                PrState::Failed
                | PrState::Pending
                | PrState::Mergeable
                | PrState::Draft
                | PrState::Merged
                | PrState::Closed => {}
            }
        }
        assert_wire(&contract, "pr_state", &pr_states);
        checked.insert("pr_state");
        let marks = [MarkKind::Raised, MarkKind::Working];
        for mark in marks {
            match mark {
                MarkKind::Raised | MarkKind::Working => {}
            }
        }
        assert_wire(&contract, "descendant_mark", &marks);
        checked.insert("descendant_mark");

        let strip_kinds = [
            StripTabKind::Herdr,
            StripTabKind::File,
            StripTabKind::Diff,
            StripTabKind::Session,
        ];
        for variant in strip_kinds {
            match variant {
                StripTabKind::Herdr
                | StripTabKind::File
                | StripTabKind::Diff
                | StripTabKind::Session => {}
            }
        }
        assert_wire(&contract, "strip_tab_kind", &strip_kinds);
        checked.insert("strip_tab_kind");

        let card_kinds = [
            MemoryCardKind::Rule,
            MemoryCardKind::Decision,
            MemoryCardKind::Lesson,
            MemoryCardKind::Fact,
            MemoryCardKind::Preference,
        ];
        for variant in card_kinds {
            match variant {
                MemoryCardKind::Rule
                | MemoryCardKind::Decision
                | MemoryCardKind::Lesson
                | MemoryCardKind::Fact
                | MemoryCardKind::Preference => {}
            }
        }
        assert_wire(&contract, "memory_card_kind", &card_kinds);
        checked.insert("memory_card_kind");

        let analysis_states = [
            MemoryAnalysisState::Idle,
            MemoryAnalysisState::Analyzing,
            MemoryAnalysisState::AiNotReady,
            MemoryAnalysisState::Checking,
            MemoryAnalysisState::CheckBudgetExhausted,
        ];
        for variant in analysis_states {
            match variant {
                MemoryAnalysisState::Idle
                | MemoryAnalysisState::Analyzing
                | MemoryAnalysisState::AiNotReady
                | MemoryAnalysisState::Checking
                | MemoryAnalysisState::CheckBudgetExhausted => {}
            }
        }
        assert_wire(&contract, "memory_analysis_state", &analysis_states);
        checked.insert("memory_analysis_state");

        let promotion_states = [
            MemoryPromotionState::Running,
            MemoryPromotionState::PrOpen,
            MemoryPromotionState::Merged,
            MemoryPromotionState::Failed,
            MemoryPromotionState::Unconfirmed,
            MemoryPromotionState::Building,
            MemoryPromotionState::Unbuilt,
        ];
        for variant in promotion_states {
            match variant {
                MemoryPromotionState::Running
                | MemoryPromotionState::PrOpen
                | MemoryPromotionState::Merged
                | MemoryPromotionState::Failed
                | MemoryPromotionState::Unconfirmed
                | MemoryPromotionState::Building
                | MemoryPromotionState::Unbuilt => {}
            }
        }
        assert_wire(&contract, "memory_promotion_state", &promotion_states);
        checked.insert("memory_promotion_state");

        let start_failures = [
            MemoryStartFailureCode::ShellStarting,
            MemoryStartFailureCode::AgentNotStarted,
            MemoryStartFailureCode::CardNotProcessed,
            MemoryStartFailureCode::Other,
        ];
        for variant in start_failures {
            match variant {
                MemoryStartFailureCode::ShellStarting
                | MemoryStartFailureCode::CardNotProcessed
                | MemoryStartFailureCode::AgentNotStarted
                | MemoryStartFailureCode::Other => {}
            }
        }
        assert_wire(&contract, "memory_start_failure_code", &start_failures);
        checked.insert("memory_start_failure_code");

        let actions = [
            MemoryAction::Enable,
            MemoryAction::Disable,
            MemoryAction::Delete,
            MemoryAction::Forget,
            MemoryAction::Undo,
            MemoryAction::Promote,
            MemoryAction::Dismiss,
            MemoryAction::CardRight,
            MemoryAction::DocRight,
        ];
        for variant in actions {
            match variant {
                MemoryAction::Enable
                | MemoryAction::Disable
                | MemoryAction::Delete
                | MemoryAction::Forget
                | MemoryAction::Undo
                | MemoryAction::Promote
                | MemoryAction::Dismiss
                | MemoryAction::CardRight
                | MemoryAction::DocRight => {}
            }
            assert_eq!(serde_json::to_value(variant).unwrap(), variant.as_str());
        }
        assert_wire(&contract, "memory_action", &actions);
        checked.insert("memory_action");

        let send_states = [
            ViewFeedbackSendState::Sending,
            ViewFeedbackSendState::Sent,
            ViewFeedbackSendState::Waiting,
            ViewFeedbackSendState::Reopening,
            ViewFeedbackSendState::Failed,
        ];
        for variant in send_states {
            match variant {
                ViewFeedbackSendState::Sending
                | ViewFeedbackSendState::Sent
                | ViewFeedbackSendState::Waiting
                | ViewFeedbackSendState::Reopening
                | ViewFeedbackSendState::Failed => {}
            }
        }
        assert_wire(&contract, "view_feedback_send_state", &send_states);
        checked.insert("view_feedback_send_state");

        let send_reasons = [
            ViewFeedbackSendReason::AttachmentsFull,
            ViewFeedbackSendReason::LetterUnsaved,
            ViewFeedbackSendReason::LettersFull,
            ViewFeedbackSendReason::ShotCopy,
            ViewFeedbackSendReason::ReopenFailed,
            ViewFeedbackSendReason::Closed,
        ];
        for variant in send_reasons {
            match variant {
                ViewFeedbackSendReason::AttachmentsFull
                | ViewFeedbackSendReason::LetterUnsaved
                | ViewFeedbackSendReason::LettersFull
                | ViewFeedbackSendReason::ShotCopy
                | ViewFeedbackSendReason::ReopenFailed
                | ViewFeedbackSendReason::Closed => {}
            }
        }
        assert_wire(&contract, "view_feedback_send_reason", &send_reasons);
        checked.insert("view_feedback_send_reason");

        let find_routes = [PaneFindRoute::Agent, PaneFindRoute::Bar];
        for variant in find_routes {
            match variant {
                PaneFindRoute::Agent | PaneFindRoute::Bar => {}
            }
        }
        assert_wire(&contract, "pane_find_route", &find_routes);
        checked.insert("pane_find_route");

        let input_states = [
            InputRequestState::Pending,
            InputRequestState::Ready,
            InputRequestState::Discarded,
        ];
        for variant in input_states {
            match variant {
                InputRequestState::Pending
                | InputRequestState::Ready
                | InputRequestState::Discarded => {}
            }
        }
        assert_wire(&contract, "input_request_state", &input_states);
        checked.insert("input_request_state");

        let kit_parts = hide_kit::ComponentId::ALL;
        for variant in kit_parts {
            match variant {
                hide_kit::ComponentId::Cli
                | hide_kit::ComponentId::ClaudeCodeHook
                | hide_kit::ComponentId::CodexHook
                | hide_kit::ComponentId::CoordinationRetirement => {}
            }
        }
        assert_wire(&contract, "kit_component_id", &kit_parts);
        checked.insert("kit_component_id");

        let kit_states = [
            hide_kit::ComponentState::Installed,
            hide_kit::ComponentState::Outdated,
            hide_kit::ComponentState::NotInstalled,
            hide_kit::ComponentState::Removed,
            hide_kit::ComponentState::Failed,
            hide_kit::ComponentState::Absent,
            hide_kit::ComponentState::Off,
        ];
        for variant in kit_states {
            match variant {
                hide_kit::ComponentState::Installed
                | hide_kit::ComponentState::Outdated
                | hide_kit::ComponentState::NotInstalled
                | hide_kit::ComponentState::Removed
                | hide_kit::ComponentState::Failed
                | hide_kit::ComponentState::Absent
                | hide_kit::ComponentState::Off => {}
            }
        }
        assert_wire(&contract, "kit_component_state", &kit_states);
        checked.insert("kit_component_state");

        let onboarding = [AgentOnboarding::Pending, AgentOnboarding::Done];
        for variant in onboarding {
            match variant {
                AgentOnboarding::Pending | AgentOnboarding::Done => {}
            }
        }
        assert_wire(&contract, "agent_onboarding", &onboarding);
        checked.insert("agent_onboarding");

        let first_run = [
            FirstRunStep::Agents,
            FirstRunStep::Notifications,
            FirstRunStep::Project,
            FirstRunStep::Done,
        ];
        for variant in first_run {
            match variant {
                FirstRunStep::Agents
                | FirstRunStep::Notifications
                | FirstRunStep::Project
                | FirstRunStep::Done => {}
            }
        }
        assert_wire(&contract, "first_run_step", &first_run);
        checked.insert("first_run_step");

        let intros = [ScreenIntro::Factory];
        for variant in intros {
            match variant {
                ScreenIntro::Factory => {}
            }
        }
        assert_wire(&contract, "screen_intro", &intros);
        checked.insert("screen_intro");

        let logo_modes = [AgentLogoMode::Shown, AgentLogoMode::Hidden];
        for variant in logo_modes {
            match variant {
                AgentLogoMode::Shown | AgentLogoMode::Hidden => {}
            }
        }
        assert_wire(&contract, "agent_logo_mode", &logo_modes);
        checked.insert("agent_logo_mode");

        let behind = [
            hide_kit::device_herdr::BehindCause::CannotHandOff,
            hide_kit::device_herdr::BehindCause::HandoffFailed,
        ];
        for variant in behind {
            match variant {
                hide_kit::device_herdr::BehindCause::CannotHandOff
                | hide_kit::device_herdr::BehindCause::HandoffFailed => {}
            }
        }
        assert_wire(&contract, "herdr_behind_cause", &behind);
        checked.insert("herdr_behind_cause");

        let kit_availability = [
            hide_kit::Availability::Available,
            hide_kit::Availability::NotInstalled,
            hide_kit::Availability::UnsupportedSystem,
        ];
        for variant in kit_availability {
            match variant {
                hide_kit::Availability::Available
                | hide_kit::Availability::NotInstalled
                | hide_kit::Availability::UnsupportedSystem => {}
            }
        }
        assert_wire(&contract, "kit_agent_availability", &kit_availability);
        checked.insert("kit_agent_availability");

        let features = hide_kit::Feature::ALL;
        for variant in features {
            match variant {
                hide_kit::Feature::Skill
                | hide_kit::Feature::Guidance
                | hide_kit::Feature::Letters
                | hide_kit::Feature::Bell
                | hide_kit::Feature::Memory
                | hide_kit::Feature::SpawnGuard
                | hide_kit::Feature::HerdrIntegration
                | hide_kit::Feature::Sleep
                | hide_kit::Feature::Fork
                | hide_kit::Feature::Start
                | hide_kit::Feature::Titles => {}
            }
        }
        assert_wire(&contract, "kit_feature", &features);
        checked.insert("kit_feature");

        let reasons = [
            PaneConnectionReason::CodexSharedServer,
            PaneConnectionReason::StartedBeforeHide,
            PaneConnectionReason::SetupNeeded,
        ];
        for variant in reasons {
            match variant {
                PaneConnectionReason::CodexSharedServer
                | PaneConnectionReason::StartedBeforeHide
                | PaneConnectionReason::SetupNeeded => {}
            }
        }
        assert_wire(&contract, "pane_connection_reason", &reasons);
        checked.insert("pane_connection_reason");

        let reopen_failures = [
            PaneReopenFailure::StartRefused,
            PaneReopenFailure::SessionGone,
            PaneReopenFailure::CodexUnread,
            PaneReopenFailure::AgentBusy,
            PaneReopenFailure::EndRefused,
        ];
        for variant in reopen_failures {
            match variant {
                PaneReopenFailure::StartRefused
                | PaneReopenFailure::SessionGone
                | PaneReopenFailure::CodexUnread
                | PaneReopenFailure::AgentBusy
                | PaneReopenFailure::EndRefused => {}
            }
        }
        assert_wire(&contract, "pane_reopen_failure", &reopen_failures);
        checked.insert("pane_reopen_failure");

        let daemon_failures = [
            CodexDaemonOffFailure::CodexMissing,
            CodexDaemonOffFailure::CodexRefused,
            CodexDaemonOffFailure::TimedOut,
            CodexDaemonOffFailure::Unreachable,
            CodexDaemonOffFailure::StopFailed,
        ];
        for variant in daemon_failures {
            match variant {
                CodexDaemonOffFailure::CodexMissing
                | CodexDaemonOffFailure::CodexRefused
                | CodexDaemonOffFailure::TimedOut
                | CodexDaemonOffFailure::Unreachable
                | CodexDaemonOffFailure::StopFailed => {}
            }
        }
        assert_wire(&contract, "codex_daemon_off_failure", &daemon_failures);
        checked.insert("codex_daemon_off_failure");

        let check_failures = [KitCheckFailure::ReadFailed];
        for variant in check_failures {
            match variant {
                KitCheckFailure::ReadFailed => {}
            }
        }
        assert_wire(&contract, "kit_check_failure", &check_failures);
        checked.insert("kit_check_failure");

        let refusal_causes = [
            hide_kit::RefusalCause::Unsafe,
            hide_kit::RefusalCause::Record,
            hide_kit::RefusalCause::Busy,
            hide_kit::RefusalCause::Projects,
            hide_kit::RefusalCause::Other,
        ];
        for variant in refusal_causes {
            match variant {
                hide_kit::RefusalCause::Unsafe
                | hide_kit::RefusalCause::Record
                | hide_kit::RefusalCause::Busy
                | hide_kit::RefusalCause::Projects
                | hide_kit::RefusalCause::Other => {}
            }
        }
        assert_wire(&contract, "kit_refusal_cause", &refusal_causes);
        checked.insert("kit_refusal_cause");

        let state_variables = [
            hide_kit::layout::StateVariable::HideStateDir,
            hide_kit::layout::StateVariable::XdgStateHome,
        ];
        for variant in state_variables {
            match variant {
                hide_kit::layout::StateVariable::HideStateDir
                | hide_kit::layout::StateVariable::XdgStateHome => {}
            }
        }
        assert_wire(&contract, "kit_state_variable", &state_variables);
        checked.insert("kit_state_variable");
        use crate::links::{FileState, IssueSource, LinkTarget, SessionRole};
        let files = [FileState::Present, FileState::Missing, FileState::Unknown];
        for variant in files {
            match variant {
                FileState::Present | FileState::Missing | FileState::Unknown => {}
            }
        }
        assert_wire(&contract, "link_file_state", &files);
        checked.insert("link_file_state");

        let sources = [IssueSource::Closes, IssueSource::Hide];
        for variant in sources {
            match variant {
                IssueSource::Closes | IssueSource::Hide => {}
            }
        }
        assert_wire(&contract, "link_issue_source", &sources);
        checked.insert("link_issue_source");

        let roles = [SessionRole::Created, SessionRole::Worked];
        for variant in roles {
            match variant {
                SessionRole::Created | SessionRole::Worked => {}
            }
        }
        assert_wire(&contract, "link_session_role", &roles);
        checked.insert("link_session_role");

        // An internally tagged enum: its `kind` is the wire string.
        let targets = [
            LinkTarget::Pr { number: 1 },
            LinkTarget::Issue { key: "k".into() },
        ];
        let kinds = targets
            .iter()
            .map(|target| {
                match target {
                    LinkTarget::Pr { .. } | LinkTarget::Issue { .. } => {}
                }
                serde_json::to_value(target).unwrap()["kind"].clone()
            })
            .collect::<Vec<_>>();
        assert_wire(&contract, "link_target_kind", &kinds);
        checked.insert("link_target_kind");

        // The Factory screens (PRD software-factory-ui): the summary carries
        // these as the engine's own strings.
        use hide_factory::model::{
            AttemptOutcome, AttemptStage, Column, DiscoveryClass, QuestionKind, QuestionOrigin,
            TaskState,
        };
        for state in TaskState::ALL {
            match state {
                TaskState::Drafting
                | TaskState::Waiting
                | TaskState::Running
                | TaskState::Paused
                | TaskState::Blocked
                | TaskState::Verifying
                | TaskState::MergeWaiting
                | TaskState::Landed
                | TaskState::Done
                | TaskState::Stopped
                | TaskState::Relanding
                | TaskState::Outside
                | TaskState::Cancelled => {}
            }
        }
        let states = TaskState::ALL.map(TaskState::as_str);
        assert_wire(&contract, "factory_task_state", &states);
        checked.insert("factory_task_state");

        for column in Column::ALL {
            match column {
                Column::Before | Column::Moving | Column::Stuck | Column::Done => {}
            }
        }
        let columns = Column::ALL.map(Column::as_str);
        assert_wire(&contract, "factory_column", &columns);
        checked.insert("factory_column");

        // An internally tagged enum: its `kind` is the wire string.
        let kinds = [
            QuestionKind::Intake,
            QuestionKind::Split { pieces: Vec::new() },
            QuestionKind::Default,
            QuestionKind::Blocking,
            QuestionKind::ScopeChange { change: None },
            QuestionKind::NewTaskCap,
            QuestionKind::ProposedTask {
                draft: Box::default(),
                discovery: None,
            },
            QuestionKind::Action,
            QuestionKind::ConfirmCard,
        ]
        .iter()
        .map(|kind| {
            match kind {
                QuestionKind::Intake
                | QuestionKind::Split { .. }
                | QuestionKind::Default
                | QuestionKind::Blocking
                | QuestionKind::ScopeChange { .. }
                | QuestionKind::NewTaskCap
                | QuestionKind::ProposedTask { .. }
                | QuestionKind::Action
                | QuestionKind::ConfirmCard => {}
            }
            serde_json::to_value(kind).unwrap()["kind"].clone()
        })
        .collect::<Vec<_>>();
        assert_wire(&contract, "factory_question_kind", &kinds);
        checked.insert("factory_question_kind");

        let origins = [
            QuestionOrigin::Review,
            QuestionOrigin::Worker,
            QuestionOrigin::Check,
            QuestionOrigin::Engine,
        ];
        for origin in origins {
            match origin {
                QuestionOrigin::Review
                | QuestionOrigin::Worker
                | QuestionOrigin::Check
                | QuestionOrigin::Engine => {}
            }
        }
        assert_wire(&contract, "factory_question_origin", &origins);
        checked.insert("factory_question_origin");

        let classes = [
            DiscoveryClass::InScope,
            DiscoveryClass::Decision,
            DiscoveryClass::ScopeChange,
            DiscoveryClass::Prerequisite,
            DiscoveryClass::Unrelated,
        ];
        for class in classes {
            match class {
                DiscoveryClass::InScope
                | DiscoveryClass::Decision
                | DiscoveryClass::ScopeChange
                | DiscoveryClass::Prerequisite
                | DiscoveryClass::Unrelated => {}
            }
        }
        assert_wire(&contract, "factory_discovery_class", &classes);
        checked.insert("factory_discovery_class");

        let stages = [AttemptStage::Task, AttemptStage::PreMerge];
        for stage in stages {
            match stage {
                AttemptStage::Task | AttemptStage::PreMerge => {}
            }
        }
        assert_wire(&contract, "factory_attempt_stage", &stages);
        checked.insert("factory_attempt_stage");

        // `AttemptView.outcome`: an outcome's tag, or `running` before one.
        let outcomes = [
            Some(AttemptOutcome::Passed),
            Some(AttemptOutcome::Failed {
                check: String::new(),
                link: String::new(),
            }),
            Some(AttemptOutcome::Environment {
                signal: String::new(),
                check: String::new(),
            }),
            Some(AttemptOutcome::Cancelled),
            None,
        ]
        .iter()
        .map(|outcome| match outcome {
            Some(
                outcome @ (AttemptOutcome::Passed
                | AttemptOutcome::Failed { .. }
                | AttemptOutcome::Environment { .. }
                | AttemptOutcome::Cancelled),
            ) => serde_json::to_value(outcome).unwrap()["result"].clone(),
            None => serde_json::json!("running"),
        })
        .collect::<Vec<_>>();
        assert_wire(&contract, "factory_attempt_outcome", &outcomes);
        checked.insert("factory_attempt_outcome");

        // The codes beside the summary's sentences; `hide-factory`'s
        // `every_summary_code_is_pinned` holds each `ALL` to every variant.
        use hide_factory::model::{EnvHold, Gate, StopReason};
        use hide_factory::summary::{ResultCode, WaitingFor};
        assert_wire(&contract, "factory_waiting_for", &WaitingFor::ALL);
        checked.insert("factory_waiting_for");
        assert_wire(&contract, "factory_env_hold", &EnvHold::ALL);
        checked.insert("factory_env_hold");
        assert_wire(&contract, "factory_stop_reason", &StopReason::ALL);
        checked.insert("factory_stop_reason");
        assert_wire(&contract, "factory_gate", &Gate::ALL);
        checked.insert("factory_gate");
        assert_wire(&contract, "factory_result_code", &ResultCode::ALL);
        checked.insert("factory_result_code");
        use hide_factory::model::{
            CriterionState, DecisionKind, DecisionSource, Fallback, FollowUpState, ObserverMode,
            PauseReason, RecoveryAction, RecoveryOutcome,
        };
        use hide_factory::summary::{DecisionBy, Holding};
        assert_wire(&contract, "factory_holding", &Holding::ALL);
        checked.insert("factory_holding");
        assert_wire(&contract, "factory_decision_by", &DecisionBy::ALL);
        checked.insert("factory_decision_by");
        assert_wire(&contract, "factory_decision_source", &DecisionSource::ALL);
        checked.insert("factory_decision_source");
        assert_wire(&contract, "factory_follow_up_state", &FollowUpState::ALL);
        checked.insert("factory_follow_up_state");
        assert_wire(&contract, "factory_criterion_state", &CriterionState::ALL);
        checked.insert("factory_criterion_state");
        assert_wire(&contract, "factory_recovery_outcome", &RecoveryOutcome::ALL);
        checked.insert("factory_recovery_outcome");
        assert_wire(&contract, "factory_fallback", &Fallback::ALL);
        checked.insert("factory_fallback");
        assert_wire(&contract, "factory_recovery_action", &RecoveryAction::ALL);
        checked.insert("factory_recovery_action");
        assert_wire(&contract, "factory_decision_kind", &DecisionKind::ALL);
        checked.insert("factory_decision_kind");
        assert_wire(&contract, "factory_pause_reason", &PauseReason::ALL);
        checked.insert("factory_pause_reason");
        assert_wire(&contract, "factory_observer_mode", &ObserverMode::ALL);
        checked.insert("factory_observer_mode");
        use hide_factory::judgment::WorkerTextSource;
        assert_wire(
            &contract,
            "factory_diagnosis_source",
            &WorkerTextSource::ALL,
        );
        checked.insert("factory_diagnosis_source");

        use crate::agent_state::tally::tasks::TaskState as OverviewTaskState;
        let task_states = [
            OverviewTaskState::NeedsYou,
            OverviewTaskState::Blocked,
            OverviewTaskState::Working,
            OverviewTaskState::Waiting,
            OverviewTaskState::Done,
            OverviewTaskState::Resting,
            OverviewTaskState::Cleanup,
        ];
        for variant in task_states {
            match variant {
                OverviewTaskState::NeedsYou
                | OverviewTaskState::Blocked
                | OverviewTaskState::Working
                | OverviewTaskState::Waiting
                | OverviewTaskState::Done
                | OverviewTaskState::Resting
                | OverviewTaskState::Cleanup => {}
            }
        }
        assert_wire(&contract, "task_state", &task_states);
        checked.insert("task_state");

        use crate::model::{BacklogFailure, BacklogPhase};
        let backlog_phases = [
            BacklogPhase::Waiting,
            BacklogPhase::Creating,
            BacklogPhase::Created,
            BacklogPhase::Failed,
        ];
        for variant in backlog_phases {
            match variant {
                BacklogPhase::Waiting
                | BacklogPhase::Creating
                | BacklogPhase::Created
                | BacklogPhase::Failed => {}
            }
        }
        assert_wire(&contract, "backlog_phase", &backlog_phases);
        checked.insert("backlog_phase");
        let backlog_failures = [
            BacklogFailure::NotInstalled,
            BacklogFailure::NotLoggedIn,
            BacklogFailure::NoGithubRemote,
            BacklogFailure::NetworkOrRateLimit,
            BacklogFailure::QueueFull,
            BacklogFailure::Unconfirmed,
            BacklogFailure::RepositoryChanged,
            BacklogFailure::Unavailable,
            BacklogFailure::Refused,
        ];
        for variant in backlog_failures {
            match variant {
                BacklogFailure::NotInstalled
                | BacklogFailure::NotLoggedIn
                | BacklogFailure::NoGithubRemote
                | BacklogFailure::NetworkOrRateLimit
                | BacklogFailure::QueueFull
                | BacklogFailure::Unconfirmed
                | BacklogFailure::RepositoryChanged
                | BacklogFailure::Unavailable
                | BacklogFailure::Refused => {}
            }
        }
        assert_wire(&contract, "backlog_failure", &backlog_failures);
        checked.insert("backlog_failure");

        let color_themes = [ColorTheme::Hide, ColorTheme::Neutral];
        for variant in color_themes {
            match variant {
                ColorTheme::Hide | ColorTheme::Neutral => {}
            }
        }
        assert_wire(&contract, "color_theme", &color_themes);
        checked.insert("color_theme");

        let unchecked = contract
            .keys()
            .filter(|key| !checked.contains(key.as_str()))
            .collect::<Vec<_>>();
        assert!(
            unchecked.is_empty(),
            "contract lists enums this test does not pin: {unchecked:?}"
        );
    }
}
