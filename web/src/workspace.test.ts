import { describe, expect, it } from "vitest";
import type { Checkout, StripTab } from "./snapshot";
import type { ViewDisplaySnapshot, ViewLayoutSnapshot } from "./snapshot";
import { agentEntries, laneFrame, laneLanding, NO_CALLS_SEEN, pageCalled, pageLaneWidth, readCalls, TOOLS, TOOL_SCOPES, treeLanding, type LaneInput, type LaneSizes } from "./workspace";

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
// `--size-lane-ideal`, `--size-tree-min`, `--size-tree-ideal`,
// `--spacing-sm`), so the widths read as the PRD and the boards state them.
const sizes: LaneSizes = { agentMin: 480, laneMin: 360, laneIdeal: 520, lanePage: 900, treeMin: 180, treeIdeal: 240, divider: 8 };

const frame = (input: Partial<Omit<LaneInput, "sizes">> & { body: number }) =>
  laneFrame({ tool: null, pinned: false, tree: false, laneWidth: null, treeWidth: null, hasViews: true, shown: false, expanded: false, ...input, sizes });

describe("the right lane (PRD right-lane)", () => {
  it("gives Agent Views the whole body while the lane is closed or put away (B1, B4)", () => {
    expect(frame({ body: 1200 })).toMatchObject({ mode: "closed", agents: 1200 });
    expect(frame({ tool: "files", body: 1200 })).toMatchObject({ mode: "hidden", agents: 1200, width: 520 });
  });

  it("draws an unpinned lane over Agent Views at its width, Agent Views keeping its own (B3)", () => {
    expect(frame({ tool: "files", shown: true, body: 1200 })).toMatchObject({ mode: "overlay", agents: 1200, width: 520, document: 520, tree: null });
    expect(frame({ tool: "prs", shown: true, laneWidth: 700, body: 1200 })).toMatchObject({ mode: "overlay", agents: 1200, width: 700 });
  });

  it("docks a pinned lane, Agent Views giving up its width and the divider's (B5, B7)", () => {
    expect(frame({ tool: "files", pinned: true, body: 1200 })).toMatchObject({ mode: "docked", agents: 1200 - 8 - 520, width: 520 });
    // The pin is the lane's, whichever tool shows and whether or not it was called.
    expect(frame({ tool: "memory", pinned: true, body: 1200 })).toMatchObject({ mode: "docked", agents: 672 });
  });

  it("keeps the stored width the same docked or drawn over, never under the lane's minimum (B6)", () => {
    expect(frame({ tool: "files", pinned: true, laneWidth: 600, body: 1400 }).width).toBe(600);
    expect(frame({ tool: "files", shown: true, laneWidth: 600, body: 1400 }).width).toBe(600);
    expect(frame({ tool: "files", shown: true, laneWidth: 100, body: 1400 }).width).toBe(360);
  });

  it("widens the lane by the tree beside a document, so the document keeps its width (B10)", () => {
    const tree = frame({ tool: "files", shown: true, tree: true, body: 1400 });
    expect(tree).toMatchObject({ width: 520 + 8 + 240, document: 520, tree: 240, treeOnly: false });
    // Docked, Agent Views gives up the tree's width too.
    expect(frame({ tool: "files", pinned: true, tree: true, body: 1400 })).toMatchObject({ mode: "docked", agents: 1400 - 8 - 768, document: 520, tree: 240 });
    // Another tool keeps the tree on without drawing it.
    expect(frame({ tool: "changes", shown: true, tree: true, body: 1400 })).toMatchObject({ width: 520, tree: null });
  });

  it("gives the tree the whole lane while no view is open, whether or not it is on (B12)", () => {
    for (const tree of [true, false]) {
      expect(frame({ tool: "files", shown: true, tree, hasViews: false, body: 1400 })).toMatchObject({ width: 520, tree: null, treeOnly: true });
    }
  });

  it("docks a pinned lane wherever its minimums fit beside Agent Views' 480, giving up its own width first, and draws it over them only below that (D-07, B19)", () => {
    // A document of 520 and a tree of 240 with their divider, beside the lane's divider and 480, want 1256; in 1255 the lane gives up 1px.
    const squeezed = frame({ tool: "files", pinned: true, tree: true, body: 1255 });
    expect(squeezed).toMatchObject({ mode: "docked", narrowPin: false, agents: 480, width: 767, tree: 240, document: 519 });
    const wide = frame({ tool: "files", pinned: true, tree: true, body: 1256 });
    expect(wide).toMatchObject({ mode: "docked", narrowPin: false, agents: 480, width: 768 });
    // Down to its minimums, 360 and 180 with their divider, the lane stays docked; one pixel less draws it over the agents.
    expect(frame({ tool: "files", pinned: true, tree: true, body: 1036 })).toMatchObject({ mode: "docked", agents: 480, width: 548, document: 360, tree: 180 });
    expect(frame({ tool: "files", pinned: true, tree: true, body: 1035 })).toMatchObject({ mode: "overlay", narrowPin: true, agents: 1035, width: 768 });
    // Without the tree the same holds at 360.
    expect(frame({ tool: "files", pinned: true, body: 1007 })).toMatchObject({ mode: "docked", agents: 480, width: 519 });
    expect(frame({ tool: "files", pinned: true, body: 848 })).toMatchObject({ mode: "docked", agents: 480, width: 360 });
    expect(frame({ tool: "files", pinned: true, body: 847 })).toMatchObject({ mode: "overlay", narrowPin: true });
    // Widening again brings the stored width back: nothing narrow is stored.
    expect(frame({ tool: "files", pinned: true, body: 1600 })).toMatchObject({ mode: "docked", width: 520 });
  });

  it("opens the lane at any width, a body narrower than it giving up document width before tree width (B19)", () => {
    expect(frame({ tool: "files", shown: true, body: 400 })).toMatchObject({ mode: "overlay", width: 400, document: 400 });
    expect(frame({ tool: "files", shown: true, tree: true, body: 700 })).toMatchObject({ width: 700, document: 452, tree: 240 });
    expect(frame({ tool: "files", shown: true, tree: true, body: 560 })).toMatchObject({ width: 560, document: 360, tree: 192 });
  });

  it("expands over the whole body while Agent Views keeps the width it had (B8)", () => {
    expect(frame({ tool: "files", shown: true, expanded: true, tree: true, body: 1400 })).toMatchObject({ mode: "expanded", agents: 1400, width: 1400, tree: 240, document: 1400 - 8 - 240 });
    expect(frame({ tool: "files", pinned: true, expanded: true, body: 1400 })).toMatchObject({ mode: "expanded", agents: 1400 - 8 - 520, width: 1400 });
  });

  it("places nothing before the body is measured, so no terminal fits to a guess", () => {
    expect(frame({ tool: "files", pinned: true, body: 0 })).toMatchObject({ agents: 0, width: 0 });
  });

  it("lands the docked lane's divider as the document width, within the minimums (B6)", () => {
    const docked = frame({ tool: "files", pinned: true, body: 1600 });
    // The divider dragged 100px left widens the lane by 100px.
    expect(laneLanding({ x: 1600 - 520 - 8 - 100, body: 1600, frame: docked, sizes })).toEqual({ lane_width: 620 });
    expect(laneLanding({ x: 0, body: 1600, frame: docked, sizes })).toEqual({ lane_width: 1600 - 8 - 480 });
    expect(laneLanding({ x: 1590, body: 1600, frame: docked, sizes })).toEqual({ lane_width: 360 });
    // With the tree beside the document, the tree keeps its width.
    const tree = frame({ tool: "files", pinned: true, tree: true, body: 1600 });
    expect(laneLanding({ x: 1600 - 768 - 8 - 32, body: 1600, frame: tree, sizes })).toEqual({ lane_width: 552 });
    // Over the agents the same drag lands the same width, and the lane may
    // reach the body's edge: nothing beside it keeps a minimum.
    const over = frame({ tool: "files", shown: true, body: 1600 });
    expect(over.mode).toBe("overlay");
    expect(laneLanding({ x: 1600 - 520 - 8 - 100, body: 1600, frame: over, sizes })).toEqual({ lane_width: 620 });
    expect(laneLanding({ x: 0, body: 1600, frame: over, sizes })).toEqual({ lane_width: 1600 - 8 });
    expect(laneLanding({ x: 1590, body: 1600, frame: over, sizes })).toEqual({ lane_width: 360 });
  });

  it("trades width between the tree and the document, so the lane and Agent Views keep theirs (B10)", () => {
    const tree = frame({ tool: "files", pinned: true, tree: true, body: 1600 });
    const landed = treeLanding({ x: 300, frame: tree, sizes });
    expect(landed).toEqual({ tree_width: 300, lane_width: 460 });
    expect(treeLanding({ x: 10, frame: tree, sizes })).toEqual({ tree_width: 180, lane_width: 580 });
    expect(treeLanding({ x: 700, frame: tree, sizes })).toEqual({ tree_width: 400, lane_width: 360 });
    // Expanded, only the tree's width is kept, so the stored document width is not the body's.
    const expanded = frame({ tool: "files", shown: true, expanded: true, tree: true, body: 1600 });
    expect(treeLanding({ x: 300, frame: expanded, sizes })).toEqual({ tree_width: 300 });
  });
});

describe("widening the lane for a page (PRD browser-width-zoom)", () => {
  // `--size-lane-page` is 900: wide enough that a site switching to its
  // desktop layout at 768 draws it.
  const widen = (laneWidth: number | null, body: number, tree: number | null = null) => pageLaneWidth({ laneWidth, tree, body, sizes });

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

  it("keeps the tree beside the document within what Agent Views leaves (B2)", () => {
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
