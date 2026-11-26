// The Workspace screen's rules (PRD S6 B5-B11, B17; PRD right-lane D-01..D-07,
// D-11; PRD lane-two-columns D-01, D-02, D-09): which strip entries are Agent
// tabs, what an Agent tab is called and marked with, and how the body is
// divided between Agent Views and the right lane's two columns. The core
// owns every stored value here (`rest.workspace_view`, the strip); these
// functions only read them, so they are testable without a page. The View
// areas' own rules are `viewLayout.ts`.

import { findArea } from "./areaLayout";
import { workspaceKey } from "./viewLayout";
import type { Checkout, DetailItem, SnapshotRest, StripTab, ViewLayoutSnapshot } from "./snapshot";

export { TOOLS, type Tool } from "./laneTools";
import type { Tool } from "./laneTools";

/** What a tool is about: the Project (the Project tool) or the front checkout (Explorer, Changes). The rail and the tool column's header draw it (PRD work-tab-pr-lens D-37). */
export type ToolScope = "project" | "checkout";
export const TOOL_SCOPES: Record<Tool, ToolScope> = { project: "project", memory: "project", explorer: "checkout", changes: "checkout" };

/**
 * What the viewing column shows (PRD lane-desk-pair-view D-06): the desk of
 * the Agent tab in front while the core says it is shown and has a tab, else
 * nothing. Read with the lane closed too, so a closed lane's shape is known.
 */
export type LaneView = "documents" | "none";

export function laneViewOf(view: Pick<WorkspaceView, "tray"> | null, hasViews: boolean): LaneView {
  return view?.tray.shown && hasViews ? "documents" : "none";
}

/** The front Workspace's presentation as the core published it. */
export type WorkspaceView = {
  device_id: string;
  path: string;
  git?: CheckoutGitHeader | null;
  /** Whether the lane is open; closed, its columns keep their shape for ⌘E (B15). */
  open: boolean;
  /** The tool column's tool, or null while there is none. */
  tool: Tool | null;
  /** The documents' width in CSS pixels, or null for the default until it is resized (D-09). */
  document_width: number | null;
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
  /** Which column the last call brought forward (B19): the viewing column for an open, a card, the desk button or ⌘E, the tool column for a tool, a reveal or a folder link. */
  lane_called_side: LaneSide;
  /** The Workspace the operator last chose, now or before a restart, so the page opens on it (PRD right-lane D-11). */
  resumed?: boolean;
  /** The desk of the Agent tab in front (PRD lane-desk-pair-view D-06): its tab, whether the lane shows it, how many Views are on it (the rail's number, D-29) and its pair view. `layout` is its View areas. */
  tray: TraySnapshot;
  /** The window's pair view mode (D-45, D-47): on or off, and the left pane's share of the body in thousandths once the divider was dragged. */
  pair_mode?: PairMode;
  /** The panes of this Workspace's Agent tabs with a View to show beside the desk, their agent's or one opened from their links (B19, D-54): the pane head draws its ◧ cell on these only (D-53). */
  panes_with_views?: string[];
  /** The View areas (S7) of the desk in front; absent only from a core that predates them. */
  layout?: ViewLayoutSnapshot;
  /** Null on a device this core dials, whose engine shows one tab. */
  agent_layout?: import("./agentLayout").AgentLayout | null;
  /** The operator's comments on this checkout's Views; absent while there are none and no Send to say. */
  feedback?: import("./snapshot").ViewFeedback;
  /** For each pane of the checkout, the Views of unsaved documents its close would close with its session (PRD session-files D-14); absent while there are none. */
  session_drafts?: SessionDrafts[];
};

/** One Agent tab's desk as the screen draws it. */
/** `floats`: the desk floats with the tool beside it over the agents, since a View came onto it while that tool floated (D-50); a shown desk that does not float stands. */
export type TraySnapshot = { tab_id: string; shown: boolean; count: number; floats?: boolean; pair?: PairView | null };
export type PairMode = { on: boolean; ratio?: number | null };
/** An Agent tab in pair view: the pane drawn beside its desk, or `alone` at the body's full width while its desk holds no tab the strip shows (D-53, D-64) or ⌘E or the rail's desk button folded it (D-57, D-63); `shown`, the core's count of the desk's tabs the strip shows, says whether the desk can stand at all; and on the desk on screen the tabs another agent of that tab owns, which fold into `Others N` (D-41). */
export type PairView = { tab_id: string; pane_id: string; alone?: boolean; shown?: number; folded?: string[] };

/** The Views of unsaved documents one pane's session alone opened. */
export type SessionDrafts = { pane_id: string; views: { display_id: string; tab_id: string }[] };

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
  /** The operator's `받기` while it runs, or why it did not; absent otherwise and from an older core. */
  catch_up?: { state: "running" } | { state: "refused"; files: number } | { state: "busy" } | { state: "failed" } | null;
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
  /** The documents' minimum and width until the operator resizes them. */
  viewMin: number;
  viewIdeal: number;
  /** The document width a page opened in the lane widens it to (`pageLaneWidth`). */
  lanePage: number;
  explorerMin: number;
  explorerIdeal: number;
  /** The Project tool and Changes. */
  panelMin: number;
  panelIdeal: number;
  /** A divider: at the lane's left edge, and between the two columns. */
  divider: number;
};

/**
 * How the lane is drawn now, a fact of this page alone and never sent (PRD
 * right-lane D-02..D-04, D-07, D-11; lane-desk-pair-view D-50):
 *
 * - `closed`: no column; Agent Views has the body.
 * - `docked`: the desk stands at the body's right and Agent Views narrows
 *   beside it; a tool opened meanwhile floats at the right edge with the
 *   desk on its left, the two one layer over the agents, which keep the
 *   width the desk left them.
 * - `overlay`: a tool, or the desk floating with it, drawn over Agent Views,
 *   which keeps its width; in a body too narrow for the desk to stand, the
 *   desk floats here too.
 * - `expanded`: over the whole body; Agent Views keeps the width it had.
 * - `pair`: the lane is the right half of the body beside the paired pane
 *   (PRD lane-desk-pair-view B17, D-16).
 */
export type LaneMode = "closed" | "docked" | "overlay" | "expanded" | "pair";

/** The Workspace widths the columns draw from (D-09). */
export type LaneWidthField = "document_width" | "explorer_width" | "panel_width";

/** Which of the two columns a rail button or a stack brings in front. */
export type LaneSide = "view" | "tool";

/** A column as it is drawn: the stored width it draws from and lands on, its width now, and the least it goes to. */
export type LaneColumn = { field: LaneWidthField; width: number; min: number };

export type LaneFrame = {
  mode: LaneMode;
  /** Agent Views' width: the body, less a standing desk and its divider; 0 before the body is measured, so no terminal fits to a guess. */
  agents: number;
  /** The lane's drawn width, both columns and their divider included. */
  width: number;
  /** The viewing column while it is drawn, else null. */
  view: LaneColumn | null;
  /** The tool column while it is drawn, else null. */
  tool: LaneColumn | null;
  /** Both columns exist but the body holds only one at a time, the front one drawn (B19). */
  stacked: boolean;
  /** In pair view, the paired pane's width left of the divider (D-47). */
  pane?: number;
};

/** What `laneFrame` reads: the core's lane and this page's own facts. */
export type LaneInput = {
  open: boolean;
  view: LaneView;
  tool: Tool | null;
  documentWidth: number | null;
  explorerWidth: number | null;
  panelWidth: number | null;
  expanded: boolean;
  /** The desk floats with the tool (`tray.floats`, D-50); else a drawn desk stands where it fits. */
  floats?: boolean;
  /** Pair view is drawn beside its desk (`pairBesideDesk`): the lane is the desk beside the paired pane, whatever the lane's own open state. */
  pair?: boolean;
  /** Pair view's divider landing, the left pane's share in thousandths (D-47); half while null. */
  pairRatio?: number | null;
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
 * The standing desk (D-50): the desk at its stored width at the body's right,
 * giving way down to its minimum so Agent Views keeps `agentMin` beside it.
 * A tool opened meanwhile floats at the right edge with the desk on its left,
 * the two laid out as a floating lane's columns, while Agent Views keeps the
 * width the desk left it, so the tool opening and closing resizes nothing.
 * Null when the desk floats with the tool, or when even its minimum would
 * leave Agent Views under `agentMin`: the desk then floats as it did before.
 */
function standing(input: LaneInput, view: Want | null, tool: Want | null): LaneFrame | null {
  const { body, sizes } = input;
  if (!view || input.floats === true || body <= 0) return null;
  const room = body - sizes.divider - sizes.agentMin;
  if (room < view.min) return null;
  const width = Math.min(view.want, room);
  const agents = body - width - sizes.divider;
  const desk = { field: view.field, width, min: view.min };
  if (!tool) return { mode: "docked", agents, width, view: desk, tool: null, stacked: false };
  // A body that stands the desk holds both columns at their minimums; one that did not would float them.
  const fit = fitColumns({ ...view, want: width }, tool, body, sizes.divider);
  if (!fit) return null;
  return { mode: "docked", agents, width: fit.view + sizes.divider + fit.tool, view: { ...desk, width: fit.view }, tool: { field: tool.field, width: fit.tool, min: tool.min }, stacked: false };
}

/**
 * The body as it is drawn (PRD lane-two-columns B1, B15, B17, B19, B20;
 * lane-desk-pair-view B35, D-50): a shown desk stands at the body's right and
 * Agent Views narrows beside it (`standing`); a tool, and a desk floating
 * with it, are drawn over Agent Views, which keeps its width. Each column draws at its own stored width, so a wide document
 * leaves the tool column as it was; a body narrower than both gives way
 * inside the lane, the viewing column first, and when even their minimums
 * do not fit the front one covers the other; a body narrower than one
 * column is the lane's whole width. A window that narrows and widens again
 * moves between these by width alone: nothing narrow is stored. Expanded,
 * the tool column keeps its width and the viewing column takes the rest.
 */
export function laneFrame(input: LaneInput): LaneFrame {
  const { body, sizes } = input;
  const view = viewWant(input);
  const tool = toolWant(input);
  const none: LaneFrame = { mode: "closed", agents: body, width: 0, view: null, tool: null, stacked: false };
  // Pair view draws the desk whatever the lane's own state (D-46): only its
  // own ways out end it, and the lane comes back as it was. The paired pane
  // keeps its width at the mode's ratio in every mode, so expanding the lane
  // over it and back resizes no terminal (B29, D-47).
  const pairing = input.pair === true && view !== null;
  if ((!input.open && !pairing) || (!view && !tool)) return none;
  const mode: LaneMode = input.expanded ? "expanded" : pairing ? "pair" : "overlay";
  if (body <= 0) return { ...none, mode, agents: 0 };
  const column = (want: Want, width: number): LaneColumn => ({ field: want.field, width, min: want.min });
  const pane = pairing ? { pane: pairPaneWidth(body, input.pairRatio ?? null, sizes) } : {};
  if (mode === "pair" && view && pane.pane !== undefined) {
    // The desk takes what the pane and the divider leave (B17, D-47); a tool
    // opened meanwhile stands beside the desk at its width (B15).
    const width = Math.max(0, body - pane.pane - sizes.divider);
    if (!tool) return { mode, agents: body, width, view: column(view, width), tool: null, stacked: false, ...pane };
    const toolWidth = Math.min(tool.want, Math.max(0, width - sizes.divider - view.min));
    return { mode, agents: body, width, view: column(view, width - sizes.divider - toolWidth), tool: column(tool, toolWidth), stacked: false, ...pane };
  }
  const stands = pairing ? null : standing(input, view, tool);
  if (mode === "expanded") {
    // The tool column keeps its width and the viewing column takes the rest
    // (B15); Agent Views under it keeps the width a standing desk left it.
    const agents = stands?.agents ?? body;
    if (view && tool) {
      const toolWidth = Math.min(tool.want, Math.max(0, body - sizes.divider - view.min));
      return { mode, agents, width: body, view: column(view, body - sizes.divider - toolWidth), tool: column(tool, toolWidth), stacked: false, ...pane };
    }
    const one = (view ?? tool)!;
    return { mode, agents, width: body, view: view ? column(one, body) : null, tool: tool ? column(one, body) : null, stacked: false, ...pane };
  }
  if (stands) return stands;
  if (view && tool) {
    const fit = fitColumns(view, tool, body, sizes.divider);
    if (fit) return { mode, agents: body, width: fit.view + sizes.divider + fit.tool, view: column(view, fit.view), tool: column(tool, fit.tool), stacked: false };
    const front = input.front === "tool" ? tool : view;
    const width = Math.min(front.want, body);
    return { mode, agents: body, width, view: front === view ? column(view, width) : null, tool: front === tool ? column(tool, width) : null, stacked: true };
  }
  const one = (view ?? tool)!;
  const width = Math.min(one.want, body);
  return { mode, agents: body, width, view: view ? column(view, width) : null, tool: tool ? column(tool, width) : null, stacked: false };
}

/** `value` within `min..max`, the minimum winning when the two cross. */
function clamp(value: number, min: number, max: number): number {
  return Math.round(Math.max(min, Math.min(value, max)));
}

/** A divider's landing: the widths it changes, each the value it should end at; pair view's divider lands the mode's ratio (D-47). */
export type LaneLanding = Partial<Record<LaneWidthField | "pair_ratio", number>>;

/** The column at the lane's left edge: the viewing column while it is drawn, else the tool column. */
export function edgeColumn(frame: LaneFrame): LaneColumn | null {
  return frame.view ?? frame.tool;
}

/**
 * Where the divider at the lane's left edge, dragged to `x` pixels from the
 * body's left edge, lands: the width of the column at that edge, in the
 * width it draws from, so the other column keeps its own, never under that
 * column's minimum (B17); over the agents it may reach the body's edge,
 * since nothing beside it gives way.
 */
export function laneLanding(input: { x: number; body: number; frame: LaneFrame; sizes: LaneSizes }): LaneLanding {
  const { x, body, frame, sizes } = input;
  const edge = edgeColumn(frame);
  if (!edge) return {};
  const other = frame.view && frame.tool ? frame.tool.width + sizes.divider : 0;
  const width = body - x - sizes.divider - other;
  // A standing desk keeps Agent Views at its minimum beside it (D-50).
  const max = frame.mode === "docked" ? body - sizes.divider - sizes.agentMin : body - sizes.divider - other;
  return { [edge.field]: clamp(width, edge.min, max) };
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
 * a lane narrower than `lanePage` widens to it, as far as
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

/** The card in front of the area in use of the desk in front (PRD lane-desk-pair-view D-13), or null. */
export function frontCard(view: Pick<WorkspaceView, "layout"> | null): DetailItem | null {
  const layout = view?.layout;
  if (!layout) return null;
  const area = findArea(layout.root, layout.active_area);
  return area?.displays.find((display) => display.id === area.active)?.card ?? null;
}

/** The View in front of the area in use when an agent opened it and the operator has not looked at it yet, by id, or null. */
export function frontUnseen(view: Pick<WorkspaceView, "layout"> | null): string | null {
  const layout = view?.layout;
  if (!layout) return null;
  const area = findArea(layout.root, layout.active_area);
  const display = area?.displays.find((row) => row.id === area.active);
  return display?.unseen ? display.id : null;
}

/** Engine tabs: the Agent area's strip, in the core's order. */
export function agentEntries(checkout: Checkout): StripTab[] {
  return checkout.strip.filter((entry) => entry.kind === "engine");
}

type CalledView = Pick<WorkspaceView, "device_id" | "path" | "layout" | "open" | "tray">;

/**
 * The page in front of the active View area of the desk the lane draws, as
 * its display and load stamp, so a page that came forward or loaded again
 * reads as new; null when a file, a diff, a new tab or nothing is there, or
 * the desk is not drawn.
 */
function frontPage(view: CalledView): string | null {
  const layout = view.layout;
  if (!layout || !view.open || !view.tray.shown) return null;
  const area = findArea(layout.root, layout.active_area);
  const display = area?.displays.find((row) => row.id === area.active);
  return display?.kind === "browser" && display.url ? `${display.id}\u0000${display.load ?? 0}` : null;
}

/** At most this many Workspaces keep the page they drew last (`DrawnPages`). */
export const DRAWN_PAGES_LIMIT = 256;

/**
 * The page each Workspace drew last in front of its desk (`frontPage`), by
 * `workspaceKey`. A lane that closes or a desk that folds leaves it, so the
 * same page drawn again is not new; at most `DRAWN_PAGES_LIMIT` Workspaces,
 * the one whose page changed longest ago dropped first.
 */
export type DrawnPages = ReadonlyMap<string, string>;

export const NO_DRAWN_PAGES: DrawnPages = new Map();

/**
 * Reads the page `view` draws in front (PRD browser-width-zoom D-04):
 * `fresh` when the Workspace did not draw that page last, or drew it at an
 * earlier load. A lane call with a fresh page widens the lane for it; the
 * same page brought back by ⌘E or the desk button is not fresh, while one an
 * agent opened as the lane was closed is, since it was never drawn
 * (lane-desk-pair-view B2).
 */
export function readDrawnPage(drawn: DrawnPages, view: CalledView | null | undefined): { drawn: DrawnPages; fresh: boolean } {
  const page = view ? frontPage(view) : null;
  if (!view || page === null) return { drawn, fresh: false };
  const key = workspaceKey(view);
  if (drawn.get(key) === page) return { drawn, fresh: false };
  const next = new Map(drawn);
  next.delete(key);
  next.set(key, page);
  if (next.size > DRAWN_PAGES_LIMIT) next.delete(next.keys().next().value!);
  return { drawn: next, fresh: true };
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

/** The ratio pair view starts at, in thousandths: half and half (D-47). */
export const PAIR_RATIO_HALF = 500;
/** The ratio range the core accepts, in thousandths. */
const PAIR_RATIO_MIN = 100;
const PAIR_RATIO_MAX = 900;

/**
 * The paired pane's width in a body `body` pixels wide (B17, D-47): its share
 * at the mode's ratio, half until the divider was dragged, each side kept
 * above its own minimum. The same whether the lane beside it is expanded or
 * not, so expanding resizes no terminal (B15, B29).
 */
export function pairPaneWidth(body: number, ratio: number | null, sizes: Pick<LaneSizes, "agentMin" | "viewMin" | "divider">): number {
  const room = Math.max(0, body - sizes.divider);
  const share = Math.floor((room * (ratio ?? PAIR_RATIO_HALF)) / 1000);
  // A body too narrow for both minimums splits at the ratio alone.
  if (sizes.agentMin + sizes.viewMin > room) return share;
  return clamp(share, sizes.agentMin, room - sizes.viewMin);
}

/**
 * Where pair view's divider, dragged to `x` pixels from the body's left edge,
 * lands (D-47): the pane's share of the body in thousandths, inside what the
 * core accepts and each side above its minimum.
 */
export function pairLanding(input: { x: number; body: number; sizes: Pick<LaneSizes, "agentMin" | "viewMin" | "divider"> }): number {
  const { x, body, sizes } = input;
  const room = body - sizes.divider;
  if (room <= 0) return PAIR_RATIO_HALF;
  const pane = sizes.agentMin + sizes.viewMin > room ? clamp(x, 0, room) : clamp(x, sizes.agentMin, room - sizes.viewMin);
  return clamp(Math.round((pane * 1000) / room), PAIR_RATIO_MIN, PAIR_RATIO_MAX);
}
