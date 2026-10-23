// The Workspace screen's rules (PRD S6 B5-B11, B17; PRD right-lane D-01..D-07,
// D-11; PRD lane-two-columns D-01, D-02, D-09): which strip entries are Agent
// tabs, what an Agent tab is called and marked with, and how the body is
// divided between Agent Views and the right lane's two columns. The core
// owns every stored value here (`rest.workspace_view`, the strip); these
// functions only read them, so they are testable without a page. The View
// areas' own rules are `viewLayout.ts`.

import { findArea } from "./areaLayout";
import type { Checkout, DetailSlot, SnapshotRest, StripTab, ViewLayoutSnapshot } from "./snapshot";

export { TOOLS, type Tool } from "./laneTools";
import type { Tool } from "./laneTools";

/** What a tool is about: the Project (the Project tool) or the front checkout (Explorer, Changes). The rail and the tool column's header draw it (PRD work-tab-pr-lens D-37). */
export type ToolScope = "project" | "checkout";
export const TOOL_SCOPES: Record<Tool, ToolScope> = { project: "project", memory: "project", explorer: "checkout", changes: "checkout" };

/**
 * What the viewing column shows (PRD lane-two-columns D-01, D-04): the 보기
 * slot while the core says it is shown and holds something, else the
 * checkout's documents while the Workspace shows them and has a view, else
 * nothing. Read with the lane closed too, so a closed lane's shape is known.
 */
export type LaneView = "documents" | "detail" | "none";

export function laneViewOf(view: Pick<WorkspaceView, "documents"> | null, detail: DetailSlot | null | undefined, hasViews: boolean): LaneView {
  if (detail?.shown && detail.items.length > 0) return "detail";
  return view?.documents && hasViews ? "documents" : "none";
}

/** The front Workspace's presentation as the core published it. */
export type WorkspaceView = {
  device_id: string;
  path: string;
  git?: CheckoutGitHeader | null;
  /** Whether the lane is open; closed, its columns keep their shape for ⌘E (B15). */
  open: boolean;
  /** Whether the viewing column shows the checkout's documents; false while it is folded or the last view closed (B11, B12). */
  documents: boolean;
  /** The tool column's tool, or null while there is none. */
  tool: Tool | null;
  /** Whether the lane is pinned beside Agent Views rather than drawn over it (PRD right-lane D-03). */
  pinned: boolean;
  /** The documents' width in CSS pixels, or null for the default until it is resized (D-09). */
  document_width: number | null;
  /** The 보기 slot's width, the same way, so a wide document leaves it as it was. */
  detail_width: number | null;
  /** The Explorer column's width, the same way. */
  explorer_width: number | null;
  /** The width the Project tool and Changes share, the same way. */
  panel_width: number | null;
  /** The core's latest width acknowledgement, absent before any named request. Never saved. */
  width_request_id?: string | null;
  /** The number of this Workspace's last lane call, 0 for none since the core started (`readCalls`). */
  lane_called: number;
  /** How many lane calls the core has numbered, in any Workspace. */
  lane_calls: number;
  /** Which column the last call brought forward (B19): the viewing column for an open, the 보기 slot or ⌘E, the tool column for a tool, a reveal or a folder link. */
  lane_called_side: LaneSide;
  /** The Workspace the operator last chose, now or before a restart, so the page opens on it (PRD right-lane D-11). */
  resumed?: boolean;
  /** The View areas (S7); absent only from a core that predates them. */
  layout?: ViewLayoutSnapshot;
  /** Null on a device this core dials, whose Herdr shows one tab. */
  agent_layout?: import("./agentLayout").AgentLayout | null;
  /** The operator's comments on this checkout's Views; absent while there are none and no Send to say. */
  feedback?: import("./snapshot").ViewFeedback;
};

/** Whole-checkout facts from its owning node, never Changes' scoped rows. */
export type CheckoutGitHeader = {
  state: "loading" | "fresh" | "stale" | "unavailable";
  /** The Git read behind `facts`; absent before the first answer. */
  observed?: import("./observed").Observation | null;
  reason: string | null;
  recent_ci?: CheckoutCi[];
  facts: {
    recent_commits?: CheckoutCommitRecord[];
    head_sha: string | null;
    subject: string | null;
    commit_unix_seconds: number | null;
    changed_files: number;
    conflicts: number;
    upstream: string | null;
    upstream_state: "none" | "gone" | "tracked";
    ahead: number | null;
    behind: number | null;
    last_fetch_unix_ms: number | null;
    /** The upstream's commits HEAD does not have yet, newest first, at most five; `behind` counts them all. */
    incoming_commits?: CheckoutCommitRecord[];
    /** The recent commits no remote-tracking ref holds. */
    unpushed_commits?: string[];
  } | null;
  ci: CheckoutCi | null;
  /** What Hide's background fetch last saw of the remote; absent from an older core. */
  remote?: { auto: boolean; observed: import("./observed").Observation | null } | null;
  /** The GitHub page of each recent or incoming commit a remote holds, made by the core. */
  commit_links?: { sha: string; url: string }[];
  /** The origin is a GitHub repository, so a commit with no page is not pushed. */
  commit_pages?: boolean;
};

export type CheckoutCommitRecord = { sha: string; subject: string; commit_unix_seconds: number | null };
export type CheckoutCi = {
  sha: string;
  /** GitHub's last answer for the commit; `observed` says how current it is (B22). */
  state: "loading" | "passing" | "pending" | "failed" | "none" | "unknown" | "unavailable";
  observed: import("./observed").Observation | null;
  url: string | null;
};

/** The pixel sizes the body is laid out with, read from their tokens (D-09). */
export type LaneSizes = {
  agentMin: number;
  /** The documents' and the 보기 slot's minimum and width until the operator resizes them. */
  viewMin: number;
  viewIdeal: number;
  /** The document width a page opened in the lane widens it to (`pageLaneWidth`). */
  lanePage: number;
  explorerMin: number;
  explorerIdeal: number;
  /** The Project tool and Changes. */
  panelMin: number;
  panelIdeal: number;
  /** A divider: between Agent Views and a pinned lane, and between the two columns. */
  divider: number;
};

/**
 * How the lane is drawn now, a fact of this page alone and never sent (PRD
 * right-lane D-02..D-04, D-07, D-11):
 *
 * - `closed`: no column; Agent Views has the body.
 * - `hidden`: a column, but an unpinned lane the operator put away with
 *   Escape; kept mounted out of sight.
 * - `overlay`: drawn over Agent Views, which keeps its width.
 * - `docked`: pinned, beside Agent Views, which gives up the lane's width.
 * - `expanded`: over the whole body; Agent Views keeps the width it had.
 */
export type LaneMode = "closed" | "hidden" | "overlay" | "docked" | "expanded";

/** The Workspace widths the columns draw from (D-09). */
export type LaneWidthField = "document_width" | "detail_width" | "explorer_width" | "panel_width";

/** Which of the two columns a rail button or a stack brings in front. */
export type LaneSide = "view" | "tool";

/** A column as it is drawn: the stored width it draws from and lands on, its width now, and the least it goes to. */
export type LaneColumn = { field: LaneWidthField; width: number; min: number };

export type LaneFrame = {
  mode: LaneMode;
  /** Agent Views' width; 0 before the body is measured, so no terminal fits to a guess. */
  agents: number;
  /** The lane's drawn width, both columns and their divider included. */
  width: number;
  /** The viewing column while it is drawn, else null. */
  view: LaneColumn | null;
  /** The tool column while it is drawn, else null. */
  tool: LaneColumn | null;
  /** Both columns exist but the body holds only one at a time, the front one drawn (B19). */
  stacked: boolean;
  /** Pinned, but drawn over Agent Views because docking would leave it under its minimum (B19). */
  narrowPin: boolean;
};

/** What `laneFrame` reads: the core's lane and this page's own facts. */
export type LaneInput = {
  open: boolean;
  view: LaneView;
  tool: Tool | null;
  pinned: boolean;
  documentWidth: number | null;
  detailWidth: number | null;
  explorerWidth: number | null;
  panelWidth: number | null;
  /** An unpinned lane is up: not put away with Escape since it was last called or opened. */
  shown: boolean;
  expanded: boolean;
  /** The column a stacked lane draws (B19); ignored while both fit. */
  front: LaneSide;
  /** The body's width left of the tool rail. */
  body: number;
  sizes: LaneSizes;
};

type Want = { field: LaneWidthField; want: number; min: number };

function viewWant(input: LaneInput): Want | null {
  const { sizes } = input;
  if (input.view === "documents") return { field: "document_width", want: Math.max(sizes.viewMin, input.documentWidth ?? sizes.viewIdeal), min: sizes.viewMin };
  if (input.view === "detail") return { field: "detail_width", want: Math.max(sizes.viewMin, input.detailWidth ?? sizes.viewIdeal), min: sizes.viewMin };
  return null;
}

function toolWant(input: LaneInput): Want | null {
  const { sizes } = input;
  if (input.tool === null) return null;
  if (input.tool === "explorer") return { field: "explorer_width", want: Math.max(sizes.explorerMin, input.explorerWidth ?? sizes.explorerIdeal), min: sizes.explorerMin };
  return { field: "panel_width", want: Math.max(sizes.panelMin, input.panelWidth ?? sizes.panelIdeal), min: sizes.panelMin };
}

/**
 * Two columns in `room` pixels (B19): each at its width while both fit; else
 * the viewing column gives way first, down to its minimum, then the tool
 * column; and when even the two minimums with their divider do not fit, one
 * stands in front of the other.
 */
function fitColumns(view: Want, tool: Want, room: number, divider: number): { view: number; tool: number } | null {
  if (view.want + divider + tool.want <= room) return { view: view.want, tool: tool.want };
  const viewGives = Math.max(view.min, room - divider - tool.want);
  if (viewGives + divider + tool.want <= room) return { view: viewGives, tool: tool.want };
  const toolGives = Math.max(tool.min, room - divider - view.min);
  if (view.min + divider + toolGives <= room) return { view: view.min, tool: toolGives };
  return null;
}

/**
 * The body as it is drawn (PRD lane-two-columns B1, B15, B17, B19, B20).
 * Each column draws at its own stored width, the same pinned or not, so a
 * wide document leaves the tool column as it was. A pinned lane docks
 * wherever its columns' minimums fit beside Agent Views' own, the viewing
 * column giving way first; when the two do not fit even at their minimums
 * the front one covers the other; and only a body too narrow for even that
 * draws it over the agents, where it stays (PRD right-lane D-07). A window
 * that narrows and widens again moves between these by width alone: nothing
 * narrow is stored. Expanded, the tool column keeps its width and the
 * viewing column takes the rest.
 */
export function laneFrame(input: LaneInput): LaneFrame {
  const { body, sizes } = input;
  const view = viewWant(input);
  const tool = toolWant(input);
  const columns = (view ? 1 : 0) + (tool ? 1 : 0);
  const open = input.open && columns > 0;
  const visible = open && (input.pinned || input.shown || input.expanded);
  const none: LaneFrame = { mode: "closed", agents: body, width: 0, view: null, tool: null, stacked: false, narrowPin: false };
  if (!open) return none;
  if (body <= 0) return { ...none, mode: !visible ? "hidden" : input.expanded ? "expanded" : input.pinned ? "docked" : "overlay", agents: 0 };
  // The lane's room: beside the agents' minimum when pinned, else the body.
  const dockRoom = body - sizes.divider - sizes.agentMin;
  const column = (want: Want, width: number): LaneColumn => ({ field: want.field, width, min: want.min });
  // What the lane wants in `room`: both columns, the front one alone when
  // stacked, or the one column it has.
  // `squeezed`, a column too narrow for its minimum takes `room` anyway: a
  // body narrower than one column is the lane's whole width (B19).
  const place = (room: number, squeezed = false): { view: LaneColumn | null; tool: LaneColumn | null; stacked: boolean; width: number } | null => {
    if (view && tool) {
      const fit = fitColumns(view, tool, room, sizes.divider);
      if (fit) return { view: column(view, fit.view), tool: column(tool, fit.tool), stacked: false, width: fit.view + sizes.divider + fit.tool };
      const front = input.front === "tool" ? tool : view;
      if (front.min > room && !squeezed) return null;
      const width = Math.min(front.want, room);
      return { view: front === view ? column(view, width) : null, tool: front === tool ? column(tool, width) : null, stacked: true, width };
    }
    const one = (view ?? tool)!;
    if (one.min > room && !squeezed) return null;
    const width = Math.min(one.want, room);
    return { view: view ? column(view, width) : null, tool: tool ? column(tool, width) : null, stacked: false, width };
  };
  const docks = input.pinned && dockRoom >= 0 && place(dockRoom) !== null;
  const mode: LaneMode = !visible ? "hidden" : input.expanded ? "expanded" : docks ? "docked" : "overlay";
  if (mode === "expanded") {
    // The tool column keeps its width and the viewing column takes the rest (B15).
    if (view && tool) {
      const toolWidth = Math.min(tool.want, Math.max(0, body - sizes.divider - view.min));
      return { mode, agents: docks ? body - sizes.divider - place(dockRoom)!.width : body, width: body, view: column(view, body - sizes.divider - toolWidth), tool: column(tool, toolWidth), stacked: false, narrowPin: false };
    }
    const one = (view ?? tool)!;
    return { mode, agents: docks ? body - sizes.divider - place(dockRoom)!.width : body, width: body, view: view ? column(one, body) : null, tool: tool ? column(one, body) : null, stacked: false, narrowPin: false };
  }
  // Docked, the lane takes what it wants of the agents' spare; over them it
  // may take the body, giving way inside itself like a narrow dock, and a
  // body narrower than one column is the whole lane.
  const placed = docks ? place(dockRoom)! : place(body, true)!;
  const agents = docks ? body - sizes.divider - placed.width : body;
  return { mode, agents, width: placed.width, view: placed.view, tool: placed.tool, stacked: placed.stacked, narrowPin: input.pinned && !docks };
}

/** `value` within `min..max`, the minimum winning when the two cross. */
function clamp(value: number, min: number, max: number): number {
  return Math.round(Math.max(min, Math.min(value, max)));
}

/** A divider's landing: the widths it changes, each the value it should end at. */
export type LaneLanding = Partial<Record<LaneWidthField, number>>;

/** The column at the lane's left edge: the viewing column while it is drawn, else the tool column. */
export function edgeColumn(frame: LaneFrame): LaneColumn | null {
  return frame.view ?? frame.tool;
}

/**
 * Where the divider at the lane's left edge, dragged to `x` pixels from the
 * body's left edge, lands: the width of the column at that edge, in the
 * width it draws from, so the other column keeps its own, never under that
 * column's minimum (B17). Docked, it leaves Agent Views their own width;
 * over the agents it may reach the body's edge, since nothing beside it
 * gives way.
 */
export function laneLanding(input: { x: number; body: number; frame: LaneFrame; sizes: LaneSizes }): LaneLanding {
  const { x, body, frame, sizes } = input;
  const edge = edgeColumn(frame);
  if (!edge) return {};
  const other = frame.view && frame.tool ? frame.tool.width + sizes.divider : 0;
  const width = body - x - sizes.divider - other;
  const beside = frame.mode === "overlay" ? 0 : sizes.agentMin;
  return { [edge.field]: clamp(width, edge.min, body - sizes.divider - other - beside) };
}

/**
 * Where the divider between the two columns, dragged to `x` pixels from the
 * lane's left edge, lands: the two trade width, so the lane keeps its width
 * and no terminal resizes; each keeps its minimum. An expanded lane's
 * viewing column only takes what the tool column leaves, so there the tool
 * column's width alone is stored.
 */
export function columnLanding(input: { x: number; frame: LaneFrame }): LaneLanding {
  const { x, frame } = input;
  if (!frame.view || !frame.tool) return {};
  const pair = frame.view.width + frame.tool.width;
  const view = clamp(x, frame.view.min, pair - frame.tool.min);
  return frame.mode === "expanded" ? { [frame.tool.field]: pair - view } : { [frame.view.field]: view, [frame.tool.field]: pair - view };
}

/**
 * The document width to store when a page opens in the lane (PRD
 * browser-width-zoom D-02, D-21; lane-two-columns B18), or null to leave it:
 * a lane narrower than `lanePage` widens to it, pinned or not, as far as
 * Agent Views keeps its minimum beside the lane and its tool column; it
 * never narrows the lane.
 */
export function pageLaneWidth(input: { width: number | null; beside: number | null; body: number; sizes: LaneSizes }): number | null {
  const { body, sizes } = input;
  const stored = Math.max(sizes.viewMin, input.width ?? sizes.viewIdeal);
  if (body <= 0 || stored >= sizes.lanePage) return null;
  const beside = input.beside === null ? 0 : input.beside + sizes.divider;
  const width = Math.floor(Math.min(sizes.lanePage, body - sizes.divider - beside - sizes.agentMin));
  return width > stored ? width : null;
}

export function workspaceViewOf(rest: SnapshotRest | null): WorkspaceView | null {
  return (rest?.workspace_view as WorkspaceView | undefined) ?? null;
}

/** Herdr tabs: the Agent area's strip, in the core's order. */
export function agentEntries(checkout: Checkout): StripTab[] {
  return checkout.strip.filter((entry) => entry.kind === "herdr");
}

/**
 * The page in front of the active View area, as its Workspace, display and
 * load stamp, so a page that came forward or loaded again reads as new; null
 * when a file, a diff, a new tab or nothing is there.
 */
function frontPage(view: Pick<WorkspaceView, "device_id" | "path" | "layout"> | null | undefined): string | null {
  const layout = view?.layout;
  if (!layout) return null;
  const area = findArea(layout.root, layout.active_area);
  const display = area?.displays.find((row) => row.id === area.active);
  return display?.kind === "browser" && display.url ? `${view.device_id}\u0000${view.path}\u0000${display.id}\u0000${display.load ?? 0}` : null;
}

/**
 * Whether a lane call put a page in front (PRD browser-width-zoom D-04): the
 * active area's front display is a page that was not in front, or loaded
 * again, before the call. The rail's documents button or a reveal leaves it as it was.
 */
export function pageCalled(previous: Pick<WorkspaceView, "device_id" | "path" | "layout"> | null | undefined, view: Pick<WorkspaceView, "device_id" | "path" | "layout"> | null | undefined): boolean {
  const page = frontPage(view);
  return page !== null && page !== frontPage(previous);
}

/** What the page has read of the core's lane calls. */
export type CallsSeen = {
  /** The core's call count at the last read, or null before the first. */
  last: number | null;
};

export const NO_CALLS_SEEN: CallsSeen = { last: null };

type FrontView = Pick<WorkspaceView, "lane_called" | "lane_calls">;

/**
 * The page's reading of the front Workspace's calls: `call` when that
 * Workspace was called since the last read, the call that brought it in
 * front included. A call older than the last read is history, so an old
 * call never moves a Workspace chosen later; a core that started again
 * counts from zero, so its lower count is only read.
 */
export function readCalls(seen: CallsSeen, view: FrontView | null | undefined): { seen: CallsSeen; call: boolean } {
  if (!view) return { seen, call: false };
  const call = seen.last !== null && view.lane_calls >= seen.last && view.lane_called > seen.last;
  return { seen: seen.last === view.lane_calls ? seen : { last: view.lane_calls }, call };
}
