// Shell-local UI state the core does not own: which overlay is open, the
// pending close confirmation, and the one-line notice. Everything the core
// owns (focus, tabs, layout, sidebar visibility) stays in `store.ts`.

import { create } from "zustand";
import type { ProjectSettingsTarget } from "./projectSettings";
import type { Filter as DiskFilter } from "./diskCleanup";
import type { Relation } from "./lineage";
import type { Opening } from "./navigation";
import type { SettingsTab } from "./settings";
import type { CycleItem } from "./recent";
import type { NumberedFamily } from "./shortcuts";
import type { ViewFocusRequest, ViewWorkspace } from "./viewLayout";
import type { KeyboardOwner } from "./viewFocus";
import type { BrowserDevice } from "./browserDevice";
import { NO_CALLS_SEEN, type CallsSeen, type LaneMode, type LaneSide, type LaneView } from "./workspace";
import type { Tool } from "./laneTools";

export type { ViewFocusRequest } from "./viewLayout";

/**
 * What the Workspace screen draws of the lane now, which the rail, the
 * chords and the close shortcut read: what the viewing column shows and the
 * tool column's tool, each `none`/null while that column is not drawn, a
 * stacked lane's back column included (PRD lane-two-columns B19).
 */
export type DrawnLane = { mode: LaneMode; view: LaneView; tool: Tool | null; stacked: boolean };

/**
 * A close the operator is being asked about. The sheet holds only what is
 * being closed: every snapshot re-derives whether it is the Stop-work sheet
 * or the subtree sheet, who is listed and in what state, and the sheet
 * closes by itself only when the target is gone (PRD close-agent-subtree
 * B13, B28, D-20, D-39, D-40).
 */
export type PendingClose = {
  kind: "pane" | "tab";
  id: string;
  /** The SSH device the pane or tab is on, or null for this machine. */
  targetId: string | null;
};

/**
 * The scope the center shows, picked in the sidebar (PRD S6 D-02): All
 * projects (`main`) lists every Project, `overview` is one Project, and
 * Workspace is the front checkout. It is this page's own navigation, like
 * the sidebar's mode: every value it shows is the core's, and the Workspace
 * it shows is the core's front checkout.
 * `null` until the first snapshot decides where the page starts (D-11).
 * `main` is one device's all-projects Overview, the screen its Home row opens
 * (PRD home-device-rail D-13): `deviceId` names the device, and without one it
 * is the device in front.
 */
export type Screen = { kind: "main"; deviceId?: string } | { kind: "workspace" };

/**
 * The Overview's five tabs share the selected device and project, except
 * account usage. Work is the first visit; reopening remembers the last tab.
 */
export type MainView = "work" | "projects" | "backlog" | "history" | "usage";

export const MAIN_VIEWS: readonly MainView[] = ["work", "projects", "backlog", "history", "usage"];

/** The panel open beside an Overview tab: a session's record at Home (B16; PRD sidebar-task-lens B17), a backlog item (B31) or a project (B57). */
export type OverviewPanel =
  | { kind: "record"; session: import("./snapshot").WorkSession; locator?: { request_offset: number; query?: string }; returnTo?: MainView }
  | { kind: "item"; workspaceId: string; taskKey: string }
  | { kind: "project"; workspaceId: string };

/**
 * The Overview's page state (PRD overview-device-tasks): the device shown, the
 * project the Tasks and backlog tabs are filtered to, the open panel, and each
 * tab's own filters. Every value it draws is the core's; nothing here is stored.
 */
export type OverviewPlace = {
  /** The device the Overview shows; null follows the rail's front device (D-02, D-17). */
  deviceId: string | null;
  /** The backlog shows this project alone (B5); cleared when the device changes. */
  projectId: string | null;
  panel: OverviewPanel | null;
  backlogQuery: string;
  backlogLabels: readonly string[];
  /** The Projects rows stand by disk size (B52). */
  bySize: boolean;
  /** Folds the operator opened, by key (`more:<project>`, `quiet`). */
  open: readonly string[];
};

/**
 * A pane's `개요에서 보기` (PRD pane-open-in-overview D-02): the session the
 * Overview's Work map opens chosen, named by the device, the pane and the native
 * session the pane showed when it was pressed. The map takes it once; any other
 * opening clears it, so ⌘⇧O only highlights (D-03).
 */
export type OverviewReveal = { deviceId: string; paneId: string; sessionId: string };

export const OVERVIEW_ENTRY: OverviewPlace = { deviceId: null, projectId: null, panel: null, backlogQuery: "", backlogLabels: [], bySize: false, open: [] };

/** `folds` with `fold` opened, or closed again when it was open. */
export function toggledFold(folds: readonly string[], fold: string): string[] {
  return folds.includes(fold) ? folds.filter((open) => open !== fold) : [...folds, fold];
}

export type Overlay = "none" | "shortcuts" | "find" | "add_project" | "file_palette" | "diff_palette" | "search" | "settings" | "project_settings";

/**
 * A notice the operator can act on: `refreshable` offers `refresh_status`
 * (activity unknown), and `dontSave` offers closing a view whose document
 * holds unsaved work this page cannot save, without saving it (S7 B5).
 */
export type Notice = {
  text: string;
  refreshable: boolean;
  dontSave?: { workspace: ViewWorkspace; displayId: string };
};

/** A project or checkout management dialog, named by the row that opened it. */
export type WorkspaceDialog =
  | { kind: "new_worktree"; workspaceId: string }
  /** Work started from an issue: a worktree named for it, its agent and first prompt. */
  | { kind: "start_issue"; workspaceId: string; taskKey: string }
  /** A pull request linked to an issue of its project's source (PRD overview-lenses-prs B10, B11). */
  | { kind: "pr_link"; workspaceId: string; prNumber: number; issueKey: string }
  /** A new issue made from a pull request's title and body, then linked (B12, B13). */
  | { kind: "pr_new_issue"; workspaceId: string; prNumber: number }
  /** A pull request handed to an agent on its branch (B15, B16, B18). */
  | { kind: "pr_delegate"; workspaceId: string; prNumber: number; /** `review`: the agent reads the pull request and reports to the operator (PRD work-tab-pr-lens B35). */ intent?: "review" }
  | { kind: "purpose"; workspaceId: string; checkoutId: string }
  | { kind: "delete_worktree"; workspaceId: string; checkoutId: string }
  /** The disk cleanup sheet of a local Git project, opened on a filter (PRD disk-layers B1-B3). */
  | { kind: "disk_cleanup"; workspaceId: string; filter: DiskFilter }
  | { kind: "remove_project"; workspaceId: string };

/**
 * A held switcher. `area` is a View area's tabs and `agents` the Agent area's
 * panes; both compare the highlight with `originKey`, while Recent Panels and
 * Recent Projects start on the row in use. An `agents` cycle with no pane in
 * use starts at -1, before its first row.
 */
export type Cycle = {
  kind: "panels" | "projects" | "area" | "agents";
  scope?: import("./areaCycle").CycleScope;
  originKey?: string;
  items: CycleItem[];
  index: number;
};

/** The Explorer's inline name field: a new entry in `parent`, or a rename of
 * `path`. `parent` is where a create lands; a rename ignores it. */
export type ExplorerDraft = {
  kind: "file" | "folder" | "rename";
  parent: string;
  path: string;
  initial: string;
};

/** What ⌘X or ⌘C took in the Explorer, until a paste (a cut) or the next cut or copy replaces it. It names the checkout it came from, because a paste works inside one checkout only. */
export type ExplorerClipboard = {
  mode: "copy" | "cut";
  path: string;
  isDirectory: boolean;
  root: string;
  device: string;
  /** Copy also wrote the absolute path to the system clipboard for a terminal. */
  terminalCopy?: boolean;
};

/** A failure the page itself found - a paste across checkouts, a drop past a cap or one hided refused - drawn in the Explorer's failure line under `path`, a folder. The core's own failures arrive in `explorer_operation`. */
export type ExplorerNotice = { path: string; message: string };

/** The trash confirmation: what is about to move, and the row the tree selects
 * once it is gone. */
export type PendingTrash = {
  path: string;
  name: string;
  isDirectory: boolean;
  selectAfter: string;
  /** The row's inode when the prompt opened: the host refuses an item that replaced it. */
  inode: string | null;
  /**
   * The checkout and device the prompt was opened for. A device's front
   * checkout follows its own Herdr focus and can move while the prompt is
   * open, so the confirmation goes to this target, not the one in front then (B34).
   */
  target: { root: string; device_id?: string };
};

type UiStore = {
  screen: Screen | null;
  /** Overview is a shell layer; opening it leaves the Workspace mounted. */
  overviewOpen: boolean;
  overview: OverviewPlace;
  overviewReturnFocus: HTMLElement | null;
  overviewReveal: OverviewReveal | null;
  /** ⌘⇧M's input (PRD overview-device-tasks B37-B42), and where the keyboard goes back to when it closes. */
  quickAdd: { returnFocus: HTMLElement | null } | null;
  searchReturnFocus: HTMLElement | null;
  setOverviewOpen: (open: boolean) => void;
  /** Changes the Overview's page state; a new device clears the project filter and the panel (B5). */
  setOverview: (patch: Partial<OverviewPlace>) => void;
  /** The Overview's tab; the page keeps it between openings. */
  mainView: MainView;
  /** The focus asked for by a chip, a Return or a relationship Open, until another replaces it (S6 B15, B16). */
  relation: Relation | null;
  sidebarFocus: number;
  /** The raised Projects sections the operator unfolded past their cap; this page's alone, so a fresh list starts short. */
  raisedOpen: readonly string[];
  /** The 태스크 list's open folds (`rest:<group>`, `cleanup:<group>`, `archived:<group>`); page-local, opened by the operator or by entering a resting task (sidebar-task-lens B5). */
  taskFoldsOpen: readonly string[];
  /** The Explorer row the operator last touched; the core owns the opened
   * document's `selected_path`, and a reveal syncs that into here. */
  explorerSelection: string | null;
  /** Bumped by ⌘F while a document shows; the editor of `editorFindDisplay` opens its find panel. */
  editorFindRequest: number;
  /** The display the latest ⌘F is for; one display answers it, whichever else shows the document. */
  editorFindDisplay: string | null;
  /** The `pane_find_open` whose answer ⌘F waits for, until the find bar or the agent's own search takes it, with where the keyboard was when ⌘F was pressed. */
  agentFindRequest: { id: string; owner: KeyboardOwner } | null;
  viewFocusRequest: ViewFocusRequest | null;
  /** Whether the lane takes the whole body (D-04); never sent. */
  laneExpanded: boolean;
  /** The column in front while a narrow pinned lane stacks its two (PRD lane-two-columns B19): a call or the rail's top buttons bring the viewing column, a tool its own. Never sent. */
  stackFront: LaneSide;
  /** What the page has read of the core's lane calls (`laneCalls.ts`). */
  laneCallsSeen: CallsSeen;
  /**
   * The Workspace (`workspaceKey`) whose lane call put a page in front, until
   * the Workspace screen, which measures the body, widens its lane for it
   * (PRD browser-width-zoom D-04, D-06). Never sent.
   */
  pageWiden: string | null;
  /**
   * The device size each browser display shows its page at, by host key
   * (`hostKey`); a display missing here fits its area (PRD browser-width-zoom
   * D-11). Page-local: never sent, and gone with a reload of the shell.
   */
  browserDevices: Record<string, BrowserDevice>;
  /** What the Workspace screen draws of the lane now. */
  drawnLane: DrawnLane;
  /** The Explorer's inline name field, or null. */
  explorerDraft: ExplorerDraft | null;
  /** The trash confirmation the Explorer is showing, or null. */
  pendingTrash: PendingTrash | null;
  /** The item the Explorer cut or copied, or null. */
  explorerClipboard: ExplorerClipboard | null;
  /** The failure line the page itself drew for the Explorer, or null. */
  explorerNotice: ExplorerNotice | null;
  overlay: Overlay;
  projectSettingsTarget: ProjectSettingsTarget | null;
  projectSettingsReturnFocus: HTMLElement | null;
  /** The tab the Settings sheet opens on. */
  settingsTab: SettingsTab;
  /** Settings was opened to add a device: the Devices tab opens its Add dialog once and clears this. */
  addDeviceRequested: boolean;
  /** Settings was opened from the window's strip to give up a move's wait: the Devices tab opens the move dialog at its confirm once and clears this. */
  moveGiveUpRequested: boolean;
  pendingClose: PendingClose | null;
  /** The working session whose Archive session waits for Stop work and archive (PRD session-archive B6). */
  pendingArchive: { paneId: string } | null;
  /** The archived session whose delete waits for its confirmation (B28). */
  pendingArchiveDelete: { id: string } | null;
  cycle: Cycle | null;
  /** A notice the operator can act on. */
  notice: Notice | null;
  /** The management dialog a sidebar menu opened, or null. */
  workspaceDialog: WorkspaceDialog | null;
  /** The pane whose plan chip is pinned open by its press or ⇧⌘U (PRD hide-plan B7). */
  planPane: string | null;
  /**
   * The task and removal this page started, so their later answers (an agent
   * start after the creation dialog closed, a removal that finished after its
   * dialog was hidden) are reported here and another page's are not.
   */
  watchedTask: number | null;
  /** The pane a creation made, to be focused once the snapshot lists it (B13). */
  focusWhenListed: string | null;
  /** A Workspace or agent asked for from All projects, an Overview or the Agents list, until it is in front or refused (S6 B2, B21). */
  opening: Opening | null;
  watchedRemoval: { deviceId: string; path: string; afterId: number } | null;
  /** True while a Shortcuts row is recording: the window listener then runs no command. */
  recordingShortcut: boolean;
  /**
   * The numbered family a modifier hold has revealed (`hints.ts`): the tabs
   * of the strip in front or the Agents list's rows carry their number
   * while it is set. Published by the window listener only on a change, so
   * an unrevealed hold renders nothing.
   */
  hint: NumberedFamily | null;
  /**
   * Escape handlers of the layers open inside an overlay, innermost last. The
   * window listener answers Escape with the innermost one first, so a nested
   * confirmation closes before the sheet behind it.
   */
  escapeLayers: (() => void)[];
  /**
   * The closers of the tooltips open on hover or focus (`Hint` and the
   * checkout card). Not Escape layers: a tooltip must not take the Escape a
   * focused terminal is about to receive, so its own dismiss closes it. An
   * Escape the shell consumes never reaches that dismiss, so the shell
   * closes these itself as it consumes.
   */
  tooltips: (() => void)[];
  /** A way into a screen. */
  setScreen: (screen: Screen) => void;
  /** A screen brought back as it was left (Recent Panels): Home keeps the view it had. */
  restoreScreen: (screen: Screen) => void;
  setMainView: (view: MainView) => void;
  setRelation: (relation: Relation | null) => void;
  toggleRaised: (group: string) => void;
  toggleTaskFold: (key: string) => void;
  setExplorerSelection: (path: string | null) => void;
  requestEditorFind: (displayId: string | null) => void;
  setAgentFindRequest: (request: { id: string; owner: KeyboardOwner } | null) => void;
  setViewFocusRequest: (request: ViewFocusRequest | null) => void;
  setPageWiden: (workspace: string | null) => void;
  setBrowserDevice: (key: string, device: BrowserDevice) => void;
  /** Drops the device sizes of displays the core no longer keeps (`keep` holds their host keys). */
  keepBrowserDevices: (keep: ReadonlySet<string>) => void;
  setLaneExpanded: (expanded: boolean) => void;
  setStackFront: (side: LaneSide) => void;
  setDrawnLane: (drawn: DrawnLane) => void;
  setExplorerDraft: (draft: ExplorerDraft | null) => void;
  setPendingTrash: (trash: PendingTrash | null) => void;
  setExplorerClipboard: (clipboard: ExplorerClipboard | null) => void;
  setExplorerNotice: (notice: ExplorerNotice | null) => void;
  openOverlay: (overlay: Overlay) => void;
  /** The overlay ⌘K replaced when it opened, so Settings in front reads as a screen with nothing to relate to (PRD cmdk-navigation B7). */
  searchOver: Overlay;
  closeOverlay: (overlay?: Overlay) => void;
  openSettings: (tab: SettingsTab) => void;
  openProjectSettings: (target: ProjectSettingsTarget, returnFocus?: HTMLElement | null) => void;
  /** Opens Settings on Devices with the Add device dialog in front. */
  openAddDevice: () => void;
  clearAddDeviceRequest: () => void;
  /** Opens Settings › Devices with the move dialog at its give-up confirm, from the window's strip. */
  openMoveGiveUp: () => void;
  clearMoveGiveUpRequest: () => void;
  setPendingClose: (pending: PendingClose | null) => void;
  setPendingArchive: (pending: { paneId: string } | null) => void;
  setPendingArchiveDelete: (pending: { id: string } | null) => void;
  setCycle: (cycle: Cycle | null) => void;
  setNotice: (notice: Notice | null) => void;
  setWorkspaceDialog: (dialog: WorkspaceDialog | null) => void;
  setPlanPane: (paneId: string | null) => void;
  setWatchedTask: (id: number | null) => void;
  setFocusWhenListed: (paneId: string | null) => void;
  setOpening: (opening: Opening | null) => void;
  setWatchedRemoval: (removal: { deviceId: string; path: string; afterId: number } | null) => void;
  setRecordingShortcut: (recording: boolean) => void;
  setHint: (hint: NumberedFamily | null) => void;
  /**
   * Registers an Escape layer and returns its removal. An `inline` layer (a
   * filter, a field being added, a drag, a pane's find bar) covers nothing,
   * so the close chords pass it (`coveringLayer`).
   */
  pushEscape: (handler: () => void, base?: boolean, inline?: boolean) => () => void;
  /** Registers an open tooltip's close and returns its removal. */
  pushTooltip: (close: () => void) => () => void;
};

/** The Escape layers registered `inline`: they answer Escape but cover nothing. */
const inlineLayers = new WeakSet<() => void>();

/** The innermost open layer that covers the screen (a menu, popover, dialog or sheet), or null. */
export function coveringLayer(layers: readonly (() => void)[]): (() => void) | null {
  return [...layers].reverse().find((layer) => !inlineLayers.has(layer)) ?? null;
}

export const useUiStore = create<UiStore>((set, get) => ({
  screen: null,
  overviewOpen: false,
  overview: OVERVIEW_ENTRY,
  overviewReturnFocus: null,
  overviewReveal: null,
  quickAdd: null,
  searchReturnFocus: null,
  // A pane's 개요에서 보기 belongs to the opening it was pressed for: closing the Overview or moving it to another device drops one the map never took.
  setOverviewOpen: (overviewOpen) => set({ overviewOpen, ...(overviewOpen ? {} : { overviewReveal: null }) }),
  setOverview: (patch) => {
    const current = get().overview;
    const moved = patch.deviceId !== undefined && patch.deviceId !== current.deviceId;
    set({ overview: { ...current, ...(moved ? { projectId: null, panel: null, open: [] } : {}), ...patch }, ...(moved ? { overviewReveal: null } : {}) });
  },
  mainView: "work",
  relation: null,
  sidebarFocus: 0,
  raisedOpen: [],
  taskFoldsOpen: [],
  explorerSelection: null,
  editorFindRequest: 0,
  editorFindDisplay: null,
  agentFindRequest: null,
  viewFocusRequest: null,
  laneExpanded: false,
  stackFront: "view",
  laneCallsSeen: NO_CALLS_SEEN,
  pageWiden: null,
  browserDevices: {},
  drawnLane: { mode: "closed", view: "none", tool: null, stacked: false },
  explorerDraft: null,
  pendingTrash: null,
  explorerClipboard: null,
  explorerNotice: null,
  overlay: "none",
  projectSettingsTarget: null,
  projectSettingsReturnFocus: null,
  settingsTab: "general",
  addDeviceRequested: false,
  moveGiveUpRequested: false,
  pendingClose: null,
  pendingArchive: null,
  pendingArchiveDelete: null,
  cycle: null,
  notice: null,
  workspaceDialog: null,
  planPane: null,
  watchedTask: null,
  focusWhenListed: null,
  opening: null,
  watchedRemoval: null,
  recordingShortcut: false,
  hint: null,
  escapeLayers: [],
  tooltips: [],
  // Moving by hand drops an open still waiting for its Workspace, so a late
  // answer does not pull the screen away from where the operator went.
  setScreen: (screen) => set({ screen, opening: null, overviewOpen: false, overviewReveal: null }),
  restoreScreen: (screen) => set({ screen, opening: null, overviewOpen: false, overviewReveal: null }),
  setMainView: (mainView) => set({ mainView }),
  setRelation: (relation) => set({ relation }),
  toggleRaised: (group) => {
    const open = get().raisedOpen;
    set({ raisedOpen: open.includes(group) ? open.filter((row) => row !== group) : [...open, group] });
  },
  toggleTaskFold: (key) => {
    const open = get().taskFoldsOpen;
    set({ taskFoldsOpen: open.includes(key) ? open.filter((row) => row !== key) : [...open, key] });
  },
  setExplorerSelection: (explorerSelection) => set({ explorerSelection }),
  requestEditorFind: (displayId) => set({ editorFindRequest: get().editorFindRequest + 1, editorFindDisplay: displayId }),
  setAgentFindRequest: (agentFindRequest) => set({ agentFindRequest }),
  setViewFocusRequest: (viewFocusRequest) => set({ viewFocusRequest }),
  setPageWiden: (pageWiden) => {
    if (get().pageWiden !== pageWiden) set({ pageWiden });
  },
  setBrowserDevice: (key, device) => {
    const devices = get().browserDevices;
    if ((devices[key] ?? "fit") === device) return;
    const next = { ...devices };
    if (device === "fit") delete next[key];
    else next[key] = device;
    set({ browserDevices: next });
  },
  keepBrowserDevices: (keep) => {
    const devices = get().browserDevices;
    const gone = Object.keys(devices).filter((key) => !keep.has(key));
    if (gone.length === 0) return;
    const next = { ...devices };
    for (const key of gone) delete next[key];
    set({ browserDevices: next });
  },
  setLaneExpanded: (laneExpanded) => {
    if (get().laneExpanded !== laneExpanded) set({ laneExpanded });
  },
  setStackFront: (stackFront) => {
    if (get().stackFront !== stackFront) set({ stackFront });
  },
  setDrawnLane: (drawn) => {
    const current = get().drawnLane;
    if (current.mode !== drawn.mode || current.view !== drawn.view || current.tool !== drawn.tool || current.stacked !== drawn.stacked) set({ drawnLane: drawn });
  },
  setExplorerDraft: (explorerDraft) => set({ explorerDraft }),
  setPendingTrash: (pendingTrash) => set({ pendingTrash }),
  setExplorerClipboard: (explorerClipboard) => set({ explorerClipboard }),
  setExplorerNotice: (explorerNotice) => set({ explorerNotice }),
  searchOver: "none",
  openOverlay: (overlay) => set((state) => (overlay === "search" ? { overlay, searchOver: state.overlay, searchReturnFocus: document.activeElement instanceof HTMLElement ? document.activeElement : null } : { overlay })),
  closeOverlay: (overlay) => {
    if (!overlay || get().overlay === overlay) set({ overlay: "none" });
  },
  openSettings: (settingsTab) => set({ overlay: "settings", settingsTab }),
  openProjectSettings: (projectSettingsTarget, returnFocus) => set({ overlay: "project_settings", projectSettingsTarget, projectSettingsReturnFocus: returnFocus ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null) }),
  openAddDevice: () => set({ overlay: "settings", settingsTab: "devices", addDeviceRequested: true }),
  clearAddDeviceRequest: () => set({ addDeviceRequested: false }),
  openMoveGiveUp: () => set({ overlay: "settings", settingsTab: "devices", moveGiveUpRequested: true }),
  clearMoveGiveUpRequest: () => set({ moveGiveUpRequested: false }),
  setPendingClose: (pendingClose) => set({ pendingClose }),
  setPendingArchive: (pendingArchive) => set({ pendingArchive }),
  setPendingArchiveDelete: (pendingArchiveDelete) => set({ pendingArchiveDelete }),
  setCycle: (cycle) => set({ cycle }),
  setNotice: (notice) => set({ notice }),
  setWorkspaceDialog: (workspaceDialog) => set({ workspaceDialog }),
  setPlanPane: (planPane) => set({ planPane }),
  setWatchedTask: (watchedTask) => set({ watchedTask }),
  setFocusWhenListed: (focusWhenListed) => set({ focusWhenListed }),
  setOpening: (opening) => set({ opening }),
  setWatchedRemoval: (watchedRemoval) => set({ watchedRemoval }),
  setRecordingShortcut: (recordingShortcut) => set({ recordingShortcut }),
  setHint: (hint) => {
    if (get().hint !== hint) set({ hint });
  },
  pushEscape: (handler, base = false, inline = false) => {
    if (inline) inlineLayers.add(handler);
    set({ escapeLayers: base ? [handler, ...get().escapeLayers] : [...get().escapeLayers, handler] });
    return () => set({ escapeLayers: get().escapeLayers.filter((layer) => layer !== handler) });
  },
  pushTooltip: (close) => {
    set({ tooltips: [...get().tooltips, close] });
    return () => set({ tooltips: get().tooltips.filter((tooltip) => tooltip !== close) });
  },
}));
