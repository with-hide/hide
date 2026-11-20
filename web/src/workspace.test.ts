import { describe, expect, it } from "vitest";
import type { Checkout, StripTab } from "./snapshot";
import type { ViewDisplaySnapshot, ViewLayoutSnapshot } from "./snapshot";
import { agentEntries, columnLanding, laneFrame, laneLanding, laneViewOf, DRAWN_PAGES_LIMIT, NO_CALLS_SEEN, NO_DRAWN_PAGES, pageLaneWidth, pairLanding, readDrawnPage, readCalls, TOOLS, TOOL_SCOPES, type DrawnPages, type LaneColumn, type LaneInput, type LaneSizes } from "./workspace";

const strip: StripTab[] = [
  { id: "engine:1", kind: "engine", source_id: "t1", label: "1", preview: false },
  { id: "file:a", kind: "file", source_id: "f-a", label: "a.md", preview: true },
  { id: "engine:2", kind: "engine", source_id: "t2", label: "2", preview: false },
  { id: "diff:b", kind: "diff", source_id: "d-b", label: "b.rs (working diff)", preview: false },
];

const checkout = { id: "c1", strip, tabs: [], active_tab_id: "t1" } as unknown as Checkout;

describe("the Workspace strips", () => {
  it("takes the Agent tabs from the core strip in the core's order", () => {
    expect(agentEntries(checkout).map((entry) => entry.source_id)).toEqual(["t1", "t2"]);
  });
});

// The token values (`--size-agent-views-min`, `--size-lane-min`,
// `--size-lane-ideal`, `--size-lane-page`, `--size-tree-min`,
// `--size-tree-ideal`, `--size-lane-panel-min`, `--size-lane-panel-ideal`,
// `--spacing-sm`), so the widths read as the PRD states them (D-09).
const sizes: LaneSizes = { agentMin: 480, viewMin: 360, viewIdeal: 520, lanePage: 900, explorerMin: 180, explorerIdeal: 240, panelMin: 360, panelIdeal: 400, divider: 8 };

const frame = (input: Partial<Omit<LaneInput, "sizes">> & { body: number }) =>
  laneFrame({ open: true, view: "none", tool: null, documentWidth: null, explorerWidth: null, panelWidth: null, expanded: false, front: "view", ...input, sizes });

const column = (field: LaneColumn["field"], width: number, min: number): LaneColumn => ({ field, width, min });

describe("the right lane's two columns (PRD lane-two-columns)", () => {
  it("gives Agent Views the whole body while the lane is closed or has no column (B1)", () => {
    expect(frame({ open: false, view: "documents", tool: "explorer", body: 1200 })).toMatchObject({ mode: "closed", agents: 1200, width: 0 });
    expect(frame({ view: "none", tool: null, body: 1200 })).toMatchObject({ mode: "closed", agents: 1200 });
  });

  it("draws the lane over Agent Views with both columns at their widths, Agent Views keeping its own (B20)", () => {
    expect(frame({ view: "documents", body: 1200 })).toMatchObject({ mode: "overlay", agents: 1200, width: 520, view: column("document_width", 520, 360), tool: null });
    expect(frame({ view: "documents", tool: "explorer", body: 1200 })).toMatchObject({ mode: "overlay", agents: 1200, width: 520 + 8 + 240, view: column("document_width", 520, 360), tool: column("explorer_width", 240, 180) });
    expect(frame({ tool: "project", panelWidth: 700, body: 1200 })).toMatchObject({ mode: "overlay", agents: 1200, width: 700, view: null, tool: column("panel_width", 700, 360) });
  });

  it("draws each column from its own width (B17, D-09)", () => {
    const widths = { documentWidth: 900, explorerWidth: 300, panelWidth: 440 };
    expect(frame({ view: "documents", ...widths, body: 2400 }).view).toEqual(column("document_width", 900, 360));
    expect(frame({ tool: "explorer", ...widths, body: 2400 }).tool).toEqual(column("explorer_width", 300, 180));
    for (const tool of ["project", "changes"] as const) {
      expect(frame({ tool, ...widths, body: 2400 }).tool).toEqual(column("panel_width", 440, 360));
    }
    // A wide document leaves a column never resized at its default.
    expect(frame({ tool: "project", documentWidth: 900, body: 1600 }).width).toBe(400);
    expect(frame({ view: "documents", panelWidth: 440, body: 1600 }).width).toBe(520);
  });

  it("keeps a stored width, never under the column's minimum", () => {
    expect(frame({ view: "documents", documentWidth: 600, body: 1400 }).width).toBe(600);
    expect(frame({ view: "documents", documentWidth: 100, body: 1400 }).width).toBe(360);
    expect(frame({ tool: "explorer", explorerWidth: 100, body: 1400 }).width).toBe(180);
  });

  it("opens the lane at any width, a body narrower than it narrowing the viewing column before the tool column, then stacking", () => {
    expect(frame({ view: "documents", body: 400 })).toMatchObject({ mode: "overlay", width: 400, view: column("document_width", 400, 360) });
    expect(frame({ view: "documents", tool: "explorer", body: 700 })).toMatchObject({ width: 700, view: column("document_width", 452, 360), tool: column("explorer_width", 240, 180) });
    expect(frame({ view: "documents", tool: "explorer", body: 560 })).toMatchObject({ width: 560, view: column("document_width", 360, 360), tool: column("explorer_width", 192, 180) });
    expect(frame({ view: "documents", tool: "explorer", body: 500 })).toMatchObject({ mode: "overlay", stacked: true, width: 500, tool: null });
  });

  it("expands over the whole body, the tool column keeping its width and the viewing column taking the rest, while Agent Views keeps the width it had (B15)", () => {
    expect(frame({ view: "documents", tool: "explorer", expanded: true, body: 1400 })).toMatchObject({ mode: "expanded", agents: 1400, width: 1400, view: column("document_width", 1400 - 8 - 240, 360), tool: column("explorer_width", 240, 180) });
    expect(frame({ view: "documents", expanded: true, body: 1400 })).toMatchObject({ mode: "expanded", agents: 1400, width: 1400, view: column("document_width", 1400, 360) });
    expect(frame({ tool: "project", expanded: true, body: 1400 })).toMatchObject({ mode: "expanded", width: 1400, view: null, tool: column("panel_width", 1400, 360) });
  });

  it("places nothing before the body is measured, so no terminal fits to a guess", () => {
    expect(frame({ view: "documents", body: 0 })).toMatchObject({ mode: "overlay", agents: 0, width: 0, view: null, tool: null });
  });

  it("lands the lane's left divider as the edge column's width, within the minimums, reaching the body's edge over the agents (B17)", () => {
    const over = frame({ view: "documents", body: 1600 });
    expect(over.mode).toBe("overlay");
    // The divider dragged 100px left widens the lane by 100px.
    expect(laneLanding({ x: 1600 - 520 - 8 - 100, body: 1600, frame: over, sizes })).toEqual({ document_width: 620 });
    expect(laneLanding({ x: 0, body: 1600, frame: over, sizes })).toEqual({ document_width: 1600 - 8 });
    expect(laneLanding({ x: 1590, body: 1600, frame: over, sizes })).toEqual({ document_width: 360 });
    // With the tool column beside the documents, the tool column keeps its width.
    const both = frame({ view: "documents", tool: "explorer", body: 1600 });
    expect(laneLanding({ x: 1600 - 768 - 8 - 32, body: 1600, frame: both, sizes })).toEqual({ document_width: 552 });
    // A tool alone lands its own width.
    expect(laneLanding({ x: 1600 - 400 - 8 - 100, body: 1600, frame: frame({ tool: "project", documentWidth: 900, body: 1600 }), sizes })).toEqual({ panel_width: 500 });
  });

  it("trades width between the two columns, so the lane and Agent Views keep theirs (B17)", () => {
    const both = frame({ view: "documents", tool: "explorer", body: 1600 });
    expect(columnLanding({ x: 460, frame: both })).toEqual({ document_width: 460, explorer_width: 300 });
    expect(columnLanding({ x: 10, frame: both })).toEqual({ document_width: 360, explorer_width: 400 });
    expect(columnLanding({ x: 700, frame: both })).toEqual({ document_width: 580, explorer_width: 180 });
    // Expanded, only the tool column's width is kept, so the stored document width is not the body's.
    const expanded = frame({ view: "documents", tool: "explorer", expanded: true, body: 1600 });
    expect(columnLanding({ x: 1000, frame: expanded })).toEqual({ explorer_width: 1600 - 8 - 1000 });
    expect(columnLanding({ x: 100, frame: frame({ view: "documents", body: 1600 }) })).toEqual({});
  });

  it("reads what the viewing column shows: the front tab's desk while it is shown and has a tab (PRD lane-desk-pair-view D-06)", () => {
    const tray = (shown: boolean) => ({ tray: { tab_id: "t1", shown, count: 1 } });
    expect(laneViewOf(tray(true), true)).toBe("documents");
    expect(laneViewOf(tray(true), false)).toBe("none");
    expect(laneViewOf(tray(false), true)).toBe("none");
    expect(laneViewOf(null, true)).toBe("none");
  });
});

describe("widening the lane for a page (PRD browser-width-zoom)", () => {
  // `--size-lane-page` is 900: wide enough that a site switching to its
  // desktop layout at 768 draws it.
  const widen = (width: number | null, body: number, beside: number | null = null) => pageLaneWidth({ width, beside, body, sizes });

  it("widens a lane narrower than 900 to 900 where Agent Views keeps 480 beside it (B1)", () => {
    expect(widen(null, 1440)).toBe(900);
    expect(widen(600, 900 + 8 + 480)).toBe(900);
  });

  it("widens only as far as Agent Views keeps 480, and never narrows a lane (B2)", () => {
    expect(widen(null, 900 + 8 + 480 - 1)).toBe(899);
    expect(widen(null, 1100)).toBe(1100 - 8 - 480);
    // Too narrow a window to widen at all: the stored width stays.
    expect(widen(null, 1000)).toBeNull();
    expect(widen(null, 520 + 8 + 480)).toBeNull();
    expect(widen(900, 2000)).toBeNull();
    expect(widen(1200, 2000)).toBeNull();
  });

  it("keeps the tool column beside the documents within what Agent Views leaves (B2; lane-two-columns B18)", () => {
    expect(widen(null, 1600, 240)).toBe(1600 - 8 - 240 - 8 - 480);
    expect(widen(null, 2000, 240)).toBe(900);
  });

  it("reads a stored width under the lane's minimum as the minimum", () => {
    expect(widen(200, 360 + 8 + 480)).toBeNull();
  });

  it("does nothing before the body is measured", () => {
    expect(widen(null, 0)).toBeNull();
  });
});

describe("the page a Workspace draws in front (PRD browser-width-zoom D-04)", () => {
  const display = (id: string, kind: ViewDisplaySnapshot["kind"], url: string | null = null, load = 1) =>
    ({ id, kind, url, load, tab_id: null, path: "", label: id, committed: null, preview: false, state: "ready", reason: null }) as unknown as ViewDisplaySnapshot;
  const view = (active: string, displays: ViewDisplaySnapshot[], path = "/a", shown = true) => ({
    device_id: "local",
    path,
    open: true,
    tray: { tab_id: "t1", shown, count: displays.length },
    layout: { root: { area: { id: "area-1", active, displays } }, active_area: "area-1", limits: { areas: 4, depth: 2, displays: 32 }, display_count: displays.length } as unknown as ViewLayoutSnapshot,
  });
  const file = display("d1", "file");
  const page = display("d2", "browser", "http://localhost:5173/");
  type Called = Parameters<typeof readDrawnPage>[1];
  /** Whether the last of `views`, read one after another, draws a fresh page. */
  const freshAfter = (...views: Called[]) => views.reduce<{ drawn: DrawnPages; fresh: boolean }>((read, next) => readDrawnPage(read.drawn, next), { drawn: NO_DRAWN_PAGES, fresh: false }).fresh;

  it("is a page that came in front, or one that loaded again in front (B1)", () => {
    expect(freshAfter(view("d1", [file]), view("d2", [file, page]))).toBe(true);
    expect(freshAfter(view("d2", [file, page]), view("d2", [file, { ...page, load: 2 }]))).toBe(true);
    expect(freshAfter(null, view("d2", [page]))).toBe(true);
    // The same page in front of another Workspace is new to this one.
    expect(freshAfter(view("d2", [page], "/b"), view("d2", [page]))).toBe(true);
    // A page that waited on a folded desk, never drawn, is new when the desk comes up (lane-desk-pair-view B2).
    expect(freshAfter(view("d2", [file, page], "/a", false), view("d2", [file, page]))).toBe(true);
  });

  it("is nothing when the page drawn last comes back, or a file, a diff or a new tab came in front (B4)", () => {
    expect(freshAfter(view("d2", [file, page]), view("d2", [file, page]))).toBe(false);
    expect(freshAfter(view("d2", [file, page]), view("d1", [file, page]))).toBe(false);
    // Through a file tab and back, a folded desk, or a closed lane, the page drawn last is not new.
    expect(freshAfter(view("d2", [file, page]), view("d1", [file, page]), view("d2", [file, page]))).toBe(false);
    expect(freshAfter(view("d2", [file, page]), view("d2", [file, page], "/a", false), view("d2", [file, page]))).toBe(false);
    expect(freshAfter(view("d1", [file]), view("d3", [file, display("d3", "browser")]))).toBe(false);
    expect(freshAfter(view("d1", [file]), view("d4", [file, display("d4", "diff")]))).toBe(false);
  });

  it("reads only the active area's front display", () => {
    const split = (active_area: string, other: ViewDisplaySnapshot) => ({
      device_id: "local",
      path: "/a",
      open: true,
      tray: { tab_id: "t1", shown: true, count: 2 },
      layout: { root: { split: { id: "s", axis: "row", ratio: 0.5, first: { area: { id: "area-1", active: "d1", displays: [file] } }, second: { area: { id: "area-2", active: other.id, displays: [other] } } } }, active_area, limits: { areas: 4, depth: 2, displays: 32 }, display_count: 2 } as unknown as ViewLayoutSnapshot,
    });
    // A page loading again in an area the call did not land in widens nothing.
    expect(freshAfter(split("area-1", page), split("area-1", { ...page, load: 2 }))).toBe(false);
    expect(freshAfter(split("area-1", page), split("area-2", page))).toBe(true);
  });

  it("keeps the pages of the 256 Workspaces whose page changed last", () => {
    let drawn: DrawnPages = NO_DRAWN_PAGES;
    for (let index = 0; index <= DRAWN_PAGES_LIMIT; index += 1) drawn = readDrawnPage(drawn, view("d2", [page], `/w${index}`)).drawn;
    expect(drawn.size).toBe(DRAWN_PAGES_LIMIT);
    expect(readDrawnPage(drawn, view("d2", [page], `/w${DRAWN_PAGES_LIMIT}`)).fresh).toBe(false);
    expect(readDrawnPage(drawn, view("d2", [page], "/w0")).fresh).toBe(true);
  });
});

describe("readCalls", () => {
  const view = (path: string, lane_called: number, lane_calls: number) => ({ device_id: "local", path, lane_called, lane_calls });

  it("takes the first count as history, then shows the lane for a call to the Workspace in front", () => {
    const first = readCalls(NO_CALLS_SEEN, view("/a", 3, 5));
    expect(first.call).toBe(false);
    const same = readCalls(first.seen, view("/a", 3, 5));
    expect(same.call).toBe(false);
    const called = readCalls(same.seen, view("/a", 6, 6));
    expect(called.call).toBe(true);
    expect(readCalls(called.seen, view("/a", 6, 6)).call).toBe(false);
  });

  it("leaves the front Workspace alone for a call into another, and shows the one a call brings in front", () => {
    const start = readCalls(NO_CALLS_SEEN, view("/a", 0, 2)).seen;
    const elsewhere = readCalls(start, view("/a", 0, 3));
    expect(elsewhere.call).toBe(false);
    // That older call does not move its Workspace when it is chosen later.
    expect(readCalls(elsewhere.seen, view("/b", 3, 3)).call).toBe(false);
    // A reveal from the Overview: the called Workspace comes in front with the call.
    const away = readCalls(elsewhere.seen, null).seen;
    expect(readCalls(away, view("/c", 4, 4)).call).toBe(true);
  });

  it("counts again from a core that started again", () => {
    const old = readCalls(NO_CALLS_SEEN, view("/a", 9, 9)).seen;
    const restarted = readCalls(old, view("/a", 0, 0));
    expect(restarted.call).toBe(false);
    expect(readCalls(restarted.seen, view("/a", 1, 1)).call).toBe(true);
    // A core first read at zero that starts again at zero still counts its calls.
    const fresh = readCalls(readCalls(NO_CALLS_SEEN, view("/a", 0, 0)).seen, view("/a", 3, 3)).seen;
    const again = readCalls(fresh, view("/a", 0, 0)).seen;
    expect(readCalls(again, view("/a", 1, 1)).call).toBe(true);
  });
});

describe("the rail's tool scopes (PRD work-tab-pr-lens D-37)", () => {
  it("lists the Project's tools before the checkout's, so the rail draws one boundary", () => {
    expect(TOOLS.map((tool) => TOOL_SCOPES[tool])).toEqual(["project", "project", "checkout", "checkout"]);
  });
});

describe("pair view's split (PRD lane-desk-pair-view D-46, D-47)", () => {
  it("starts half and half, draws the desk whatever the lane's own state, and follows the mode's ratio", () => {
    expect(frame({ open: false, view: "documents", pair: true, body: 1208 })).toMatchObject({ mode: "pair", agents: 1208, pane: 604, width: 1208 - 604 - 8 });
    expect(frame({ view: "documents", pair: true, pairRatio: 600, body: 1200 })).toMatchObject({ mode: "pair", pane: 720, width: 1200 - 720 - 8 });
  });

  it("keeps each side above its minimum however far the ratio says", () => {
    expect(frame({ view: "documents", pair: true, pairRatio: 900, body: 1200 }).pane).toBe(1200 - 8 - 360);
    expect(frame({ view: "documents", pair: true, pairRatio: 100, body: 1200 }).pane).toBe(480);
  });

  it("lands a drag as the pane's share in thousandths, inside each side's minimum", () => {
    expect(pairLanding({ x: 720, body: 1200, sizes })).toBe(600);
    expect(pairLanding({ x: 10, body: 1200, sizes })).toBe(400);
    expect(pairLanding({ x: 1190, body: 1200, sizes })).toBe(693);
  });
});
