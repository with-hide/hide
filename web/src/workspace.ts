// The Workspace screen's rules (PRD S6 B5-B11, B17; PRD right-lane D-01..D-07,
// D-11): which strip entries are Agent tabs, what an Agent tab is called and
// marked with, and how the body is divided between Agent Views and the right
// lane. The core owns every stored value here (`rest.workspace_view`, the
// strip); these functions only read them, so they are testable without a
// page. The View areas' own rules are `viewLayout.ts`.

import { findArea } from "./areaLayout";
import type { Checkout, DetailSlot, SnapshotRest, StripTab, ViewLayoutSnapshot } from "./snapshot";

export { TOOLS, type Tool } from "./laneTools";
import type { Tool } from "./laneTools";

/** What a tool is about: the Project (Memory, PRs) or the front checkout (Files, Changes). The rail and the lane header draw it (PRD work-tab-pr-lens D-37). */
export type ToolScope = "project" | "checkout";
export const TOOL_SCOPES: Record<Tool, ToolScope> = { memory: "project", prs: "project", files: "checkout", changes: "checkout" };

/** The PRs tool's scope chip, kept per Workspace (PRD sidebar-task-lens D-31). `only_checkout` is the front checkout only. */
export type PrsView = { only_checkout: boolean };

/**
 * What the lane draws (PRD sidebar-task-lens D-50): the 보기 slot while the
 * core says it is shown and holds something, else the Workspace's tool.
 */
export type LaneTool = Tool | "detail";

export function laneToolOf(view: Pick<WorkspaceView, "tool"> | null, detail: DetailSlot | null | undefined): LaneTool | null {
  if (detail?.shown && detail.items.length > 0) return "detail";
  return view?.tool ?? null;
}

/** The front Workspace's presentation as the core published it. */
export type WorkspaceView = {
  device_id: string;
  path: string;
  /** The tool the lane shows, or null while the lane is closed. */
  tool: Tool | null;
  /** The tool the lane last showed, kept while it is closed; null when it never showed one. */
  last_tool: Tool | null;
  /** The PRs tool's scope chip. */
  prs: PrsView;
  /** Whether the lane is pinned beside Agent Views rather than drawn over it (D-03). */
  pinned: boolean;
  /** Whether the files tool shows its tree column (D-05); with no view open the tree shows anyway. */
  tree: boolean;
  /** The files tool's document width in CSS pixels, or null for the default until it is resized. */
  lane_width: number | null;
  /** The other tools' and the 보기 slot's width, the same way, so a wide document leaves them as they were. */
  panel_width: number | null;
  /** The tree column's width, the same way. */
  tree_width: number | null;
  /** The core's latest width acknowledgement, absent before any named request. Never saved. */
  width_request_id?: string | null;
  /** The number of this Workspace's last lane call, 0 for none since the core started (`readCalls`). */
  lane_called: number;
  /** How many lane calls the core has numbered, in any Workspace. */
  lane_calls: number;
  /** The Workspace the operator last chose, now or before a restart, so the page opens on it (D-11). */
  resumed?: boolean;
  /** The View areas (S7); absent only from a core that predates them. */
  layout?: ViewLayoutSnapshot;
  /** Null on a device this core dials, whose Herdr shows one tab. */
  agent_layout?: import("./agentLayout").AgentLayout | null;
  /** The operator's comments on this checkout's Views; absent while there are none and no Send to say. */
  feedback?: import("./snapshot").ViewFeedback;
};

/** The pixel sizes the body is laid out with, read from their tokens. */
export type LaneSizes = {
  agentMin: number;
  laneMin: number;
  /** The lane's document width until the operator resizes it. */
  laneIdeal: number;
  /** The document width a page opened in the lane widens it to (`pageLaneWidth`). */
  lanePage: number;
  treeMin: number;
  /** The tree column's width until the operator resizes it. */
  treeIdeal: number;
  /** A divider: between Agent Views and a pinned lane, and between the tree and the document. */
  divider: number;
};

/**
 * How the lane is drawn now, a fact of this page alone and never sent
 * (D-02..D-04, D-07, D-11):
 *
 * - `closed`: no tool; Agent Views has the body.
 * - `hidden`: a tool, but an unpinned lane the operator put away with
 *   Escape; kept mounted out of sight.
 * - `overlay`: drawn over Agent Views, which keeps its width.
 * - `docked`: pinned, beside Agent Views, which gives up the lane's width.
 * - `expanded`: over the whole body; Agent Views keeps the width it had.
 */
export type LaneMode = "closed" | "hidden" | "overlay" | "docked" | "expanded";

/** The Workspace width a lane's document draws from: the files tool's, or the panel width every other tool shares. */
export type LaneWidthField = "lane_width" | "panel_width";

export function laneWidthField(tool: LaneTool | null): LaneWidthField {
  return tool === "files" ? "lane_width" : "panel_width";
}

export type LaneFrame = {
  mode: LaneMode;
  /** The stored width this lane draws from and its divider lands on. */
  field: LaneWidthField;
  /** Agent Views' width; 0 before the body is measured, so no terminal fits to a guess. */
  agents: number;
  /** The lane's drawn width, its tree column included. */
  width: number;
  /** The document's width beside the tree, or the lane's whole width without one. */
  document: number;
  /** The tree column's width while it shows beside a document, else null. */
  tree: number | null;
  /** The files tool with no view open: the tree takes the lane (B12). */
  treeOnly: boolean;
  /** Pinned, but drawn over Agent Views because docking would leave it under its minimum (B19). */
  narrowPin: boolean;
};

/** What `laneFrame` reads: the core's lane and this page's own facts. */
export type LaneInput = {
  tool: LaneTool | null;
  pinned: boolean;
  tree: boolean;
  laneWidth: number | null;
  panelWidth: number | null;
  treeWidth: number | null;
  /** The files tool has a view, or a file of this checkout opening into one. */
  hasViews: boolean;
  /** An unpinned lane is up: not put away with Escape since it was last called or opened. */
  shown: boolean;
  expanded: boolean;
  /** The body's width left of the tool rail. */
  body: number;
  sizes: LaneSizes;
};

/**
 * The body as it is drawn. The files tool draws at its document width and
 * every other tool at the panel width, each the same pinned or not (B6); a
 * tree beside a document adds its own width, so the document keeps its
 * width (B10). A pinned lane docks wherever its minimums fit beside Agent
 * Views' own, giving up its width first, and only a body too narrow for even
 * those draws it over them, where it stays (D-07); a window that narrows and
 * widens again moves between the two by width alone.
 */
export function laneFrame(input: LaneInput): LaneFrame {
  const { tool, body, sizes } = input;
  const treeOnly = tool === "files" && !input.hasViews;
  const treeBeside = tool === "files" && input.hasViews && input.tree;
  const field = laneWidthField(tool);
  const documentWant = Math.max(sizes.laneMin, (field === "lane_width" ? input.laneWidth : input.panelWidth) ?? sizes.laneIdeal);
  const treeWant = Math.max(sizes.treeMin, input.treeWidth ?? sizes.treeIdeal);
  const want = documentWant + (treeBeside ? treeWant + sizes.divider : 0);
  const visible = tool !== null && (input.pinned || input.shown || input.expanded);
  // A pinned lane docks wherever its minimums and Agent Views' fit side by
  // side, giving up its own width first; only a body too narrow for even
  // those draws it over the agents (D-07).
  const least = sizes.laneMin + (treeBeside ? sizes.treeMin + sizes.divider : 0);
  const docks = input.pinned && body - sizes.divider - least >= sizes.agentMin;
  const mode: LaneMode = tool === null ? "closed" : !visible ? "hidden" : input.expanded ? "expanded" : docks ? "docked" : "overlay";
  if (body <= 0) return { mode, field, agents: 0, width: 0, document: 0, tree: null, treeOnly, narrowPin: false };
  const docked = docks ? Math.min(want, body - sizes.divider - sizes.agentMin) : 0;
  const agents = docks ? body - sizes.divider - docked : body;
  const width = mode === "expanded" ? body : docks ? docked : Math.min(want, body);
  let tree: number | null = null;
  if (treeBeside) {
    // Expanded, the document takes the body the tree leaves; otherwise a
    // lane wider than the body gives up document width first, then tree width.
    const document = mode === "expanded" ? width - sizes.divider - treeWant : clamp(width - sizes.divider - treeWant, Math.min(sizes.laneMin, width), documentWant);
    tree = Math.max(0, Math.min(treeWant, width - sizes.divider - document));
  }
  const document = tree === null ? width : width - sizes.divider - tree;
  return { mode, field, agents: tool === null ? body : agents, width, document, tree, treeOnly, narrowPin: input.pinned && !docks && tool !== null };
}

/** `value` within `min..max`, the minimum winning when the two cross. */
function clamp(value: number, min: number, max: number): number {
  return Math.round(Math.max(min, Math.min(value, max)));
}

/** A divider's landing: the widths it changes, each the value it should end at. */
export type LaneLanding = Partial<Record<LaneWidthField | "tree_width", number>>;

/**
 * Where the divider at the lane's left edge, dragged to `x` pixels from the
 * body's left edge, lands: the document's width, in the width the lane draws
 * from, so the tree keeps its own, never under the lane's minimum. Docked,
 * it leaves Agent Views their own width; over the agents it may reach the
 * body's edge, since nothing beside it gives way.
 */
export function laneLanding(input: { x: number; body: number; frame: LaneFrame; sizes: LaneSizes }): LaneLanding {
  const { x, body, frame, sizes } = input;
  const tree = frame.tree === null ? 0 : frame.tree + sizes.divider;
  const document = body - x - sizes.divider - tree;
  const beside = frame.mode === "overlay" ? 0 : sizes.agentMin;
  return { [frame.field]: clamp(document, sizes.laneMin, body - sizes.divider - tree - beside) };
}

/**
 * The document width to store when a page opens in the lane (PRD
 * browser-width-zoom D-02, D-21), or null to leave it: a lane narrower than
 * `lanePage` widens to it, pinned or not, as far as Agent Views keeps its
 * minimum beside the lane and its tree; it never narrows the lane.
 */
export function pageLaneWidth(input: { laneWidth: number | null; tree: number | null; body: number; sizes: LaneSizes }): number | null {
  const { body, sizes } = input;
  const stored = Math.max(sizes.laneMin, input.laneWidth ?? sizes.laneIdeal);
  if (body <= 0 || stored >= sizes.lanePage) return null;
  const tree = input.tree === null ? 0 : input.tree + sizes.divider;
  const width = Math.floor(Math.min(sizes.lanePage, body - sizes.divider - tree - sizes.agentMin));
  return width > stored ? width : null;
}

/**
 * Where the divider between the tree and the document, dragged to `x`
 * pixels from the lane's left edge, lands: the two trade width, so the lane
 * keeps its width and no terminal resizes; each keeps its minimum. An
 * expanded lane's document only takes what the tree leaves, so there the
 * tree's width alone is stored.
 */
export function treeLanding(input: { x: number; frame: LaneFrame; sizes: LaneSizes }): LaneLanding {
  const { x, frame, sizes } = input;
  const pair = (frame.tree ?? 0) + frame.document;
  const tree = clamp(x, sizes.treeMin, pair - sizes.laneMin);
  return frame.mode === "expanded" ? { tree_width: tree } : { tree_width: tree, [frame.field]: pair - tree };
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
 * again, before the call. The rail's Files or a reveal leaves it as it was.
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
