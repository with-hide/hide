import { describe, expect, it } from "vitest";
import type { Checkout, StripTab } from "./snapshot";
import type { ViewDisplaySnapshot, ViewLayoutSnapshot } from "./snapshot";
import type { DetailSlot } from "./snapshot";
import { agentEntries, columnLanding, laneFrame, laneLanding, laneViewOf, NO_CALLS_SEEN, pageCalled, pageLaneWidth, readCalls, TOOLS, TOOL_SCOPES, type LaneColumn, type LaneInput, type LaneSizes } from "./workspace";

const strip: StripTab[] = [
  { id: "herdr:1", kind: "herdr", source_id: "t1", label: "1", preview: false },
  { id: "file:a", kind: "file", source_id: "f-a", label: "a.md", preview: true },
  { id: "herdr:2", kind: "herdr", source_id: "t2", label: "2", preview: false },
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
  laneFrame({ open: true, view: "none", tool: null, pinned: false, documentWidth: null, detailWidth: null, explorerWidth: null, panelWidth: null, shown: false, expanded: false, front: "view", ...input, sizes });

const column = (field: LaneColumn["field"], width: number, min: number): LaneColumn => ({ field, width, min });

describe("the right lane's two columns (PRD lane-two-columns)", () => {
  it("gives Agent Views the whole body while the lane is closed, has no column or is put away (B1, PRD right-lane B4)", () => {
    expect(frame({ open: false, view: "documents", tool: "explorer", body: 1200 })).toMatchObject({ mode: "closed", agents: 1200, width: 0 });
    expect(frame({ view: "none", tool: null, body: 1200 })).toMatchObject({ mode: "closed", agents: 1200 });
    expect(frame({ view: "documents", body: 1200 })).toMatchObject({ mode: "hidden", agents: 1200, width: 520 });
  });

  it("draws an unpinned lane over Agent Views with both columns at their widths, Agent Views keeping its own (B20)", () => {
    expect(frame({ view: "documents", shown: true, body: 1200 })).toMatchObject({ mode: "overlay", agents: 1200, width: 520, view: column("document_width", 520, 360), tool: null });
    expect(frame({ view: "documents", tool: "explorer", shown: true, body: 1200 })).toMatchObject({ mode: "overlay", agents: 1200, width: 520 + 8 + 240, view: column("document_width", 520, 360), tool: column("explorer_width", 240, 180) });
    expect(frame({ tool: "project", shown: true, panelWidth: 700, body: 1200 })).toMatchObject({ mode: "overlay", agents: 1200, width: 700, view: null, tool: column("panel_width", 700, 360) });
  });

  it("draws each column from its own width, docked or drawn over (B17, D-09)", () => {
    const widths = { documentWidth: 900, detailWidth: 600, explorerWidth: 300, panelWidth: 440 };
    for (const placed of [{ shown: true }, { pinned: true }]) {
      expect(frame({ view: "documents", ...placed, ...widths, body: 2400 }).view).toEqual(column("document_width", 900, 360));
      expect(frame({ view: "detail", ...placed, ...widths, body: 2400 }).view).toEqual(column("detail_width", 600, 360));
      expect(frame({ tool: "explorer", ...placed, ...widths, body: 2400 }).tool).toEqual(column("explorer_width", 300, 180));
      for (const tool of ["project", "changes"] as const) {
        expect(frame({ tool, ...placed, ...widths, body: 2400 }).tool).toEqual(column("panel_width", 440, 360));
      }
    }
    // A wide document leaves a column never resized at its default.
    expect(frame({ tool: "project", shown: true, documentWidth: 900, body: 1600 }).width).toBe(400);
    expect(frame({ view: "detail", shown: true, documentWidth: 900, body: 1600 }).width).toBe(520);
    expect(frame({ view: "documents", shown: true, panelWidth: 440, body: 1600 }).width).toBe(520);
  });

  it("docks a pinned lane, Agent Views giving up both columns' width and the dividers' (B21, PRD right-lane B5)", () => {
    expect(frame({ view: "documents", pinned: true, body: 1200 })).toMatchObject({ mode: "docked", agents: 1200 - 8 - 520, width: 520 });
    expect(frame({ view: "documents", tool: "explorer", pinned: true, body: 1400 })).toMatchObject({ mode: "docked", agents: 1400 - 8 - 768, width: 768 });
    // The pin is the lane's, whichever column shows.
    expect(frame({ tool: "project", pinned: true, body: 1200 })).toMatchObject({ mode: "docked", agents: 1200 - 8 - 400 });
  });

  it("keeps a stored width the same docked or drawn over, never under the column's minimum", () => {
    expect(frame({ view: "documents", pinned: true, documentWidth: 600, body: 1400 }).width).toBe(600);
    expect(frame({ view: "documents", shown: true, documentWidth: 600, body: 1400 }).width).toBe(600);
    expect(frame({ view: "documents", shown: true, documentWidth: 100, body: 1400 }).width).toBe(360);
    expect(frame({ tool: "explorer", shown: true, explorerWidth: 100, body: 1400 }).width).toBe(180);
  });

  it("narrows a pinned lane's viewing column first, then its tool column, down to their minimums (B19)", () => {
    // 520 + 8 + 240 beside 8 + 480 wants 1256; in 1255 the documents give up 1px.
    expect(frame({ view: "documents", tool: "explorer", pinned: true, body: 1256 })).toMatchObject({ mode: "docked", agents: 480, width: 768, view: column("document_width", 520, 360), tool: column("explorer_width", 240, 180) });
    expect(frame({ view: "documents", tool: "explorer", pinned: true, body: 1255 })).toMatchObject({ mode: "docked", narrowPin: false, agents: 480, width: 767, view: column("document_width", 519, 360), tool: column("explorer_width", 240, 180) });
    // The documents at 360, then the Explorer gives way.
    expect(frame({ view: "documents", tool: "explorer", pinned: true, body: 1096 })).toMatchObject({ mode: "docked", agents: 480, view: column("document_width", 360, 360), tool: column("explorer_width", 240, 180) });
    expect(frame({ view: "documents", tool: "explorer", pinned: true, body: 1095 })).toMatchObject({ mode: "docked", agents: 480, view: column("document_width", 360, 360), tool: column("explorer_width", 239, 180) });
    // Both at their minimums, 360 + 8 + 180 beside 488, is 1036.
    expect(frame({ view: "documents", tool: "explorer", pinned: true, body: 1036 })).toMatchObject({ mode: "docked", agents: 480, width: 548, stacked: false });
  });

  it("stacks a pinned lane's columns when even their minimums do not fit, the front one drawn, and draws it over the agents only below one column's minimum (B19)", () => {
    // Stacked, the lane takes only the front column's width, and the agents the rest.
    const stacked = frame({ view: "documents", tool: "explorer", pinned: true, body: 1035 });
    expect(stacked).toMatchObject({ mode: "docked", stacked: true, agents: 1035 - 8 - 520, width: 520, view: column("document_width", 520, 360), tool: null });
    expect(frame({ view: "documents", tool: "explorer", pinned: true, front: "tool", body: 1035 })).toMatchObject({ mode: "docked", stacked: true, agents: 1035 - 8 - 240, width: 240, view: null, tool: column("explorer_width", 240, 180) });
    // Stacked, the front column still gives way down to its own minimum.
    expect(frame({ view: "documents", tool: "explorer", pinned: true, body: 848 })).toMatchObject({ mode: "docked", stacked: true, agents: 480, width: 360 });
    expect(frame({ view: "documents", tool: "explorer", pinned: true, body: 847 })).toMatchObject({ mode: "overlay", narrowPin: true, agents: 847 });
    // Without a tool column the same holds at 360.
    expect(frame({ view: "documents", pinned: true, body: 848 })).toMatchObject({ mode: "docked", agents: 480, width: 360 });
    expect(frame({ view: "documents", pinned: true, body: 847 })).toMatchObject({ mode: "overlay", narrowPin: true });
    // A body narrower than one column is the lane's whole width, never wider than the body.
    expect(frame({ view: "documents", tool: "explorer", pinned: true, body: 332 })).toMatchObject({ mode: "overlay", stacked: true, width: 332, view: column("document_width", 332, 360), tool: null });
    expect(frame({ tool: "explorer", shown: true, body: 120 })).toMatchObject({ mode: "overlay", width: 120, tool: column("explorer_width", 120, 180) });
    // Widening again brings the stored shape back: nothing narrow is stored.
    expect(frame({ view: "documents", tool: "explorer", pinned: true, body: 1600 })).toMatchObject({ mode: "docked", stacked: false, width: 768 });
  });

  it("opens an unpinned lane at any width, a body narrower than it narrowing the viewing column before the tool column, then stacking", () => {
    expect(frame({ view: "documents", shown: true, body: 400 })).toMatchObject({ mode: "overlay", width: 400, view: column("document_width", 400, 360) });
    expect(frame({ view: "documents", tool: "explorer", shown: true, body: 700 })).toMatchObject({ width: 700, view: column("document_width", 452, 360), tool: column("explorer_width", 240, 180) });
    expect(frame({ view: "documents", tool: "explorer", shown: true, body: 560 })).toMatchObject({ width: 560, view: column("document_width", 360, 360), tool: column("explorer_width", 192, 180) });
    expect(frame({ view: "documents", tool: "explorer", shown: true, body: 500 })).toMatchObject({ mode: "overlay", stacked: true, width: 500, tool: null });
  });

  it("expands over the whole body, the tool column keeping its width and the viewing column taking the rest, while Agent Views keeps the width it had (B15)", () => {
    expect(frame({ view: "documents", tool: "explorer", shown: true, expanded: true, body: 1400 })).toMatchObject({ mode: "expanded", agents: 1400, width: 1400, view: column("document_width", 1400 - 8 - 240, 360), tool: column("explorer_width", 240, 180) });
    expect(frame({ view: "documents", pinned: true, expanded: true, body: 1400 })).toMatchObject({ mode: "expanded", agents: 1400 - 8 - 520, width: 1400, view: column("document_width", 1400, 360) });
    expect(frame({ tool: "project", shown: true, expanded: true, body: 1400 })).toMatchObject({ mode: "expanded", width: 1400, view: null, tool: column("panel_width", 1400, 360) });
  });

  it("places nothing before the body is measured, so no terminal fits to a guess", () => {
    expect(frame({ view: "documents", pinned: true, body: 0 })).toMatchObject({ mode: "docked", agents: 0, width: 0, view: null, tool: null });
  });

  it("lands the lane's left divider as the edge column's width, within the minimums (B17)", () => {
    const docked = frame({ view: "documents", pinned: true, body: 1600 });
    // The divider dragged 100px left widens the lane by 100px.
    expect(laneLanding({ x: 1600 - 520 - 8 - 100, body: 1600, frame: docked, sizes })).toEqual({ document_width: 620 });
    expect(laneLanding({ x: 0, body: 1600, frame: docked, sizes })).toEqual({ document_width: 1600 - 8 - 480 });
    expect(laneLanding({ x: 1590, body: 1600, frame: docked, sizes })).toEqual({ document_width: 360 });
    // With the tool column beside the documents, the tool column keeps its width.
    const both = frame({ view: "documents", tool: "explorer", pinned: true, body: 1600 });
    expect(laneLanding({ x: 1600 - 768 - 8 - 32, body: 1600, frame: both, sizes })).toEqual({ document_width: 552 });
    // Over the agents the same drag lands the same width, and the lane may
    // reach the body's edge: nothing beside it keeps a minimum.
    const over = frame({ view: "documents", shown: true, body: 1600 });
    expect(over.mode).toBe("overlay");
    expect(laneLanding({ x: 1600 - 520 - 8 - 100, body: 1600, frame: over, sizes })).toEqual({ document_width: 620 });
    expect(laneLanding({ x: 0, body: 1600, frame: over, sizes })).toEqual({ document_width: 1600 - 8 });
    expect(laneLanding({ x: 1590, body: 1600, frame: over, sizes })).toEqual({ document_width: 360 });
    // The 보기 slot and a tool alone land their own widths.
    expect(laneLanding({ x: 1600 - 520 - 8 - 100, body: 1600, frame: frame({ view: "detail", shown: true, documentWidth: 900, body: 1600 }), sizes })).toEqual({ detail_width: 620 });
    expect(laneLanding({ x: 1600 - 400 - 8 - 100, body: 1600, frame: frame({ tool: "project", shown: true, documentWidth: 900, body: 1600 }), sizes })).toEqual({ panel_width: 500 });
  });

  it("trades width between the two columns, so the lane and Agent Views keep theirs (B17)", () => {
    const both = frame({ view: "documents", tool: "explorer", pinned: true, body: 1600 });
    expect(columnLanding({ x: 460, frame: both })).toEqual({ document_width: 460, explorer_width: 300 });
    expect(columnLanding({ x: 10, frame: both })).toEqual({ document_width: 360, explorer_width: 400 });
    expect(columnLanding({ x: 700, frame: both })).toEqual({ document_width: 580, explorer_width: 180 });
    // Expanded, only the tool column's width is kept, so the stored document width is not the body's.
    const expanded = frame({ view: "documents", tool: "explorer", shown: true, expanded: true, body: 1600 });
    expect(columnLanding({ x: 1000, frame: expanded })).toEqual({ explorer_width: 1600 - 8 - 1000 });
    expect(columnLanding({ x: 100, frame: frame({ view: "documents", shown: true, body: 1600 }) })).toEqual({});
  });

  it("reads what the viewing column shows: the 보기 slot in front, else the documents while there are any (D-04, B12)", () => {
    const detail = { shown: true, items: [{ kind: "mission", mission: "m1" }] } as unknown as DetailSlot;
    expect(laneViewOf({ documents: true }, detail, true)).toBe("detail");
    expect(laneViewOf({ documents: true }, { ...detail, shown: false }, true)).toBe("documents");
    expect(laneViewOf({ documents: true }, { shown: true, items: [] } as unknown as DetailSlot, true)).toBe("documents");
    expect(laneViewOf({ documents: true }, null, false)).toBe("none");
    expect(laneViewOf({ documents: false }, null, true)).toBe("none");
    expect(laneViewOf(null, null, true)).toBe("none");
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

describe("a lane call that puts a page in front (PRD browser-width-zoom D-04)", () => {
  const display = (id: string, kind: ViewDisplaySnapshot["kind"], url: string | null = null, load = 1) =>
    ({ id, kind, url, load, tab_id: null, path: "", label: id, committed: null, preview: false, state: "ready", reason: null }) as unknown as ViewDisplaySnapshot;
  const view = (active: string, displays: ViewDisplaySnapshot[], path = "/a") => ({
    device_id: "local",
    path,
    layout: { root: { area: { id: "area-1", active, displays } }, active_area: "area-1", limits: { areas: 4, depth: 2, displays: 32 }, display_count: displays.length } as unknown as ViewLayoutSnapshot,
  });
  const file = display("d1", "file");
  const page = display("d2", "browser", "http://localhost:5173/");

  it("is a page that came in front, or one that loaded again in front (B1)", () => {
    expect(pageCalled(view("d1", [file]), view("d2", [file, page]))).toBe(true);
    expect(pageCalled(view("d1", [file, page]), view("d2", [file, page]))).toBe(true);
    expect(pageCalled(view("d2", [file, page]), view("d2", [file, { ...page, load: 2 }]))).toBe(true);
    expect(pageCalled(null, view("d2", [page]))).toBe(true);
    // The same page in front of another Workspace is new to this one.
    expect(pageCalled(view("d2", [page], "/b"), view("d2", [page]))).toBe(true);
  });

  it("is nothing when the page in front stayed, or a file, a diff or a new tab came in front (B4)", () => {
    expect(pageCalled(view("d2", [file, page]), view("d2", [file, page]))).toBe(false);
    expect(pageCalled(view("d2", [file, page]), view("d1", [file, page]))).toBe(false);
    expect(pageCalled(view("d1", [file]), view("d3", [file, display("d3", "browser")]))).toBe(false);
    expect(pageCalled(view("d1", [file]), view("d4", [file, display("d4", "diff")]))).toBe(false);
  });

  it("reads only the active area's front display", () => {
    const split = (active_area: string, other: ViewDisplaySnapshot) => ({
      device_id: "local",
      path: "/a",
      layout: { root: { split: { id: "s", axis: "row", ratio: 0.5, first: { area: { id: "area-1", active: "d1", displays: [file] } }, second: { area: { id: "area-2", active: other.id, displays: [other] } } } }, active_area, limits: { areas: 4, depth: 2, displays: 32 }, display_count: 2 } as unknown as ViewLayoutSnapshot,
    });
    // A page loading again in an area the call did not land in widens nothing.
    expect(pageCalled(split("area-1", page), split("area-1", { ...page, load: 2 }))).toBe(false);
    expect(pageCalled(split("area-1", page), split("area-2", page))).toBe(true);
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
