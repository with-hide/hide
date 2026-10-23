import { beforeAll, describe, expect, it } from "vitest";
import type { TFunction } from "i18next";
import { createInterfaceI18n } from "./i18n/instance";
import type { ViewDisplaySnapshot, ViewLayoutSnapshot, ViewNode } from "./snapshot";
import {
  RATIO_MIN,
  besideUnavailable,
  displayIdentity,
  displayMenu,
  type ExternalReveal,
  dropTarget,
  focusRequestArrived,
  neighbourArea,
  placeKey,
  ratioForFirst,
  resizeTarget,
  revealedScroll,
  splitEligibility,
  steppedRatio,
  viewAreaStepUnavailable,
  type ViewAreaStep,
  viewGeometry,
  viewLayoutPayload,
  isViewRefusal,
  type LayoutSizes,
  type Rect,
  type TabSlot,
} from "./viewLayout";

const SIZES: LayoutSizes = { areaMinWidth: 224, areaMinHeight: 144, divider: 2, tabStrip: 32 };

function display(id: string, overrides: Partial<ViewDisplaySnapshot> = {}): ViewDisplaySnapshot {
  return { id, tab_id: `file:${id}`, path: `/repo/${id}.md`, label: `${id}.md`, kind: "file", committed: null, preview: false, state: "open", reason: null, ...overrides };
}

function area(id: string, displays: (string | ViewDisplaySnapshot)[]): ViewNode {
  const rows = displays.map((entry) => (typeof entry === "string" ? display(entry) : entry));
  return { area: { id, active: rows[0]?.id ?? null, displays: rows } };
}

function split(id: string, axis: "row" | "column", ratio: number, first: ViewNode, second: ViewNode): ViewNode {
  return { split: { id, axis, ratio, first, second } };
}

function layout(root: ViewNode, limits: Partial<ViewLayoutSnapshot["limits"]> = {}): ViewLayoutSnapshot {
  return { root, active_area: "a1", limits: { areas: 6, depth: 3, displays: 64, ...limits }, display_count: 0 };
}

const body = (width: number, height = 600): Rect => ({ x: 0, y: 0, width, height });

let t: TFunction<"translation">;
let ko: TFunction<"translation">;
beforeAll(async () => {
  t = (await createInterfaceI18n("en")).t;
  ko = (await createInterfaceI18n("ko")).t;
});

describe("geometry", () => {
  it("keeps the stored ratio while both sides get their minimum, and a divider between them", () => {
    const g = viewGeometry(split("s1", "row", 0.5, area("a1", ["d1"]), area("a2", ["d2"])), body(1002), SIZES);
    expect(g.fits).toBe(true);
    expect(g.areas.map((box) => [box.id, box.rect.x, box.rect.width])).toEqual([["a1", 0, 500], ["a2", 502, 500]]);
    expect(g.areas[0]?.bar).toEqual({ x: 0, y: 0, width: 500, height: 32 });
    expect(g.areas[0]?.content).toEqual({ x: 0, y: 32, width: 500, height: 568 });
    expect(g.dividers[0]?.rect).toEqual({ x: 500, y: 0, width: 2, height: 600 });
  });

  it("holds a side at its minimum rather than the ratio", () => {
    const g = viewGeometry(split("s1", "row", 0.15, area("a1", ["d1"]), area("a2", ["d2"])), body(1002), SIZES);
    expect(g.areas.map((box) => box.rect.width)).toEqual([224, 776]);
  });

  it("fits exactly two minimums and a divider, and no less", () => {
    const tree = split("s1", "row", 0.5, area("a1", ["d1"]), area("a2", ["d2"]));
    expect(viewGeometry(tree, body(451), SIZES).fits).toBe(true);
    expect(viewGeometry(tree, body(450), SIZES).fits).toBe(true);
    expect(viewGeometry(tree, body(449), SIZES).fits).toBe(false);
  });

  it("sums the minimums of a nested split along its axis", () => {
    const tree = split("s1", "row", 0.5, area("a1", ["d1"]), split("s2", "row", 0.5, area("a2", ["d2"]), area("a3", ["d3"])));
    expect(viewGeometry(tree, body(676), SIZES).fits).toBe(true);
    expect(viewGeometry(tree, body(675), SIZES).fits).toBe(false);
  });

  it("measures a column split by height with the height minimum", () => {
    const tree = split("s1", "column", 0.5, area("a1", ["d1"]), area("a2", ["d2"]));
    expect(viewGeometry(tree, body(300, 290), SIZES).fits).toBe(true);
    expect(viewGeometry(tree, body(300, 289), SIZES).fits).toBe(false);
  });
});

describe("dividers", () => {
  const divider = (width: number, ratio = 0.5) => viewGeometry(split("s1", "row", ratio, area("a1", ["d1"]), area("a2", ["d2"])), body(width), SIZES).dividers[0]!;

  it("lands within both minimums and the core's range", () => {
    expect(ratioForFirst(divider(1002), 100)).toBeCloseTo(0.224);
    expect(ratioForFirst(divider(1002), 900)).toBeCloseTo(0.776);
    expect(ratioForFirst(divider(3002), 300)).toBe(RATIO_MIN);
    expect(ratioForFirst(divider(1002), 600)).toBeCloseTo(0.6);
  });

  it("steps by the keyboard step and stops at the bound", () => {
    expect(steppedRatio(divider(1002, 0.5), 0.05)).toBeCloseTo(0.55);
    expect(steppedRatio(divider(1002, 0.776), 0.05)).toBeNull();
    expect(steppedRatio(divider(1002, 0.75), 0.05)).toBeCloseTo(0.776);
  });
});

describe("neighbours", () => {
  // a1 on the left; a2 above a3 on the right, a2 the smaller.
  const tree = split("s1", "row", 0.5, area("a1", ["d1"]), split("s2", "column", 0.3, area("a2", ["d2"]), area("a3", ["d3"])));

  it("finds the area across a shared edge, the widest overlap first", () => {
    expect(neighbourArea(tree, "a1", "right")).toBe("a3");
    expect(neighbourArea(tree, "a2", "left")).toBe("a1");
    expect(neighbourArea(tree, "a2", "down")).toBe("a3");
    expect(neighbourArea(tree, "a3", "up")).toBe("a2");
  });

  it("has none past the Views' own edge", () => {
    expect(neighbourArea(tree, "a1", "left")).toBeNull();
    expect(neighbourArea(tree, "a2", "right")).toBeNull();
    expect(neighbourArea(tree, "a3", "down")).toBeNull();
  });
});

describe("a tab strip's scroll", () => {
  it("moves only as far as it takes to show the shown tab whole, its left edge first when it cannot fit", () => {
    expect(revealedScroll(0, 300, 100, 120)).toBe(0);
    expect(revealedScroll(0, 300, 400, 120)).toBe(220);
    expect(revealedScroll(250, 300, 100, 120)).toBe(100);
    expect(revealedScroll(0, 100, 400, 120)).toBe(400);
  });
});

describe("split eligibility", () => {
  const two = area("a1", ["d1", "d2"]);

  it("needs two minimums and a divider along the split's axis", () => {
    const at = (width: number, height = 600) => viewGeometry(two, body(width, height), SIZES);
    expect(splitEligibility(layout(two), at(450), SIZES, "d1", "a1", "right")).toEqual({ ok: true });
    expect(splitEligibility(layout(two), at(449), SIZES, "d1", "a1", "right")).toEqual({ ok: false, reason: "This view area is too narrow to split." });
    expect(splitEligibility(layout(two), at(300, 290), SIZES, "d1", "a1", "down")).toEqual({ ok: true });
    expect(splitEligibility(layout(two), at(300, 289), SIZES, "d1", "a1", "up")).toEqual({ ok: false, reason: "This view area is too short to split." });
  });

  it("stops at the area cap and the depth cap the core publishes", () => {
    const g = viewGeometry(two, body(1200), SIZES);
    expect(splitEligibility(layout(two, { areas: 2 }), g, SIZES, "d1", "a1", "right").ok).toBe(true);
    expect(splitEligibility(layout(two, { areas: 1 }), g, SIZES, "d1", "a1", "right")).toMatchObject({ ok: false, reason: expect.stringContaining("1 view area,") });
    const nested = split("s1", "row", 0.5, area("a1", ["d1", "d2"]), area("a2", ["d3"]));
    const wide = viewGeometry(nested, body(2000), SIZES);
    expect(splitEligibility(layout(nested, { depth: 2 }), wide, SIZES, "d1", "a1", "right").ok).toBe(true);
    expect(splitEligibility(layout(nested, { depth: 1 }), wide, SIZES, "d1", "a1", "right")).toMatchObject({ ok: false, reason: expect.stringContaining("1 level deep") });
  });

  it("refuses a split that would change nothing, but not another area's view splitting a single-view area", () => {
    const tree = split("s1", "row", 0.5, area("a1", ["d1", "d2"]), area("a2", ["d3"]));
    const g = viewGeometry(tree, body(2000), SIZES);
    expect(splitEligibility(layout(tree), g, SIZES, "d3", "a2", "right")).toEqual({ ok: false, reason: "This is the only view in its area." });
    expect(splitEligibility(layout(tree), g, SIZES, "d1", "a2", "right")).toEqual({ ok: true });
  });

  it("refuses every split while the tree does not fit the window", () => {
    const tree = split("s1", "row", 0.5, area("a1", ["d1", "d2"]), area("a2", ["d3"]));
    const g = viewGeometry(tree, body(449), SIZES);
    expect(splitEligibility(layout(tree), g, SIZES, "d1", "a1", "down")).toEqual({ ok: false, reason: "The window is too small to show more view areas." });
  });

  it("lets Open to the side split the only area only where Split right could, and never blocks it with a neighbour", () => {
    const one = area("a1", ["d1"]);
    const drawnAt = (width: number) => ({ geometry: viewGeometry(one, body(width), SIZES), sizes: SIZES });
    expect(besideUnavailable(layout(one), drawnAt(450))).toBeNull();
    expect(besideUnavailable(layout(one), drawnAt(449))).toBe("This view area is too narrow to open a second view beside it.");
    expect(besideUnavailable(layout(one, { areas: 1 }), drawnAt(1200))).toEqual(expect.stringContaining("1 view area,"));
    const tree = split("s1", "row", 0.5, area("a1", ["d1"]), area("a2", ["d2"]));
    expect(besideUnavailable(layout(tree), { geometry: viewGeometry(tree, body(449), SIZES), sizes: SIZES })).toBeNull();
    expect(besideUnavailable(layout(one), null)).toBeNull();
    const empty = area("a1", []);
    expect(besideUnavailable(layout(empty), { geometry: viewGeometry(empty, body(300), SIZES), sizes: SIZES })).toBeNull();
  });
});

describe("a display's menu", () => {
  const tree = split("s1", "row", 0.5, area("a1", [display("d1", { preview: true }), "d2"]), area("a2", ["d3"]));
  const g = viewGeometry(tree, body(2000), SIZES);
  const desktop: ExternalReveal = { host: { label: "explorer.revealFinder" }, device: "local", node: "local" };
  const browser: ExternalReveal = { host: null, device: "local", node: "local" };
  const ids = (displayId: string, external = desktop) => displayMenu(layout(tree), g, SIZES, displayId, external).map((entry) => entry.id);

  it("offers Keep open only for a preview, moves only toward an area, and nothing else", () => {
    expect(ids("d1")).toEqual(["keep_open", "split_right", "split_left", "split_up", "split_down", "move_right", "copy_path", "select_in_tree", "reveal_external", "close_view"]);
    expect(ids("d2")).toEqual(["split_right", "split_left", "split_up", "split_down", "move_right", "copy_path", "select_in_tree", "reveal_external", "close_view"]);
    expect(ids("d3")).toEqual(["split_right", "split_left", "split_up", "split_down", "move_left", "copy_path", "select_in_tree", "reveal_external", "close_view"]);
  });

  it("names the in-app select and the OS file manager apart, the second only where the host has one (issue 324)", () => {
    const tail = (external: ExternalReveal) => displayMenu(layout(tree), g, SIZES, "d3", external).slice(-4).map((entry) => [entry.label, entry.unavailable, entry.separated ?? false]);
    expect(tail(desktop)).toEqual([
      ["Copy path", null, true],
      ["Select in File Tree", null, false],
      ["Reveal in Finder", null, false],
      ["Close view", null, true],
    ]);
    expect(ids("d3", browser).slice(-3)).toEqual(["copy_path", "select_in_tree", "close_view"]);
    const remote = displayMenu(layout(tree), g, SIZES, "d3", { host: { label: "explorer.revealFileExplorer" }, device: "studio", node: "local" }).find((entry) => entry.id === "reveal_external");
    expect(remote).toMatchObject({ label: "Reveal in File Explorer", unavailable: "Only for files and folders on this computer." });
    const page = split("s1", "row", 0.5, area("a1", [display("d1", { kind: "browser", path: "", url: "https://example.invalid/" }), "d2"]), area("a2", ["d3"]));
    expect(displayMenu(layout(page), g, SIZES, "d1", desktop).map((entry) => entry.id)).not.toContain("reveal_external");
    expect(displayMenu(layout(page), g, SIZES, "d1", desktop).map((entry) => entry.id)).not.toContain("select_in_tree");
  });

  it("disables what cannot land with its reason", () => {
    const menu = displayMenu(layout(tree), g, SIZES, "d3", desktop);
    expect(menu.find((entry) => entry.id === "split_right")?.unavailable).toBe("This is the only view in its area.");
    const gone = split("s1", "row", 0.5, area("a1", [display("d1", { state: "unavailable", reason: "No such file" }), "d2"]), area("a2", ["d3"]));
    const goneMenu = displayMenu(layout(gone), g, SIZES, "d1", desktop);
    expect(goneMenu.find((entry) => entry.id === "select_in_tree")?.unavailable).toBe("The file is unavailable");
    expect(goneMenu.find((entry) => entry.id === "reveal_external")?.unavailable).toBe("The file is unavailable");
  });
});

describe("the Workspace an action names (contract 4.1)", () => {
  it("carries the frame's Workspace beside the action's own fields", () => {
    expect(viewLayoutPayload({ device_id: "mini", path: "/repo" }, { action: "close", display_id: "d2", discard: true })).toEqual({
      workspace: { device_id: "mini", path: "/repo" },
      action: "close",
      display_id: "d2",
      discard: true,
    });
  });

  it("tells the operator every View refusal, except one for a Workspace no longer in front", () => {
    expect(isViewRefusal({ kind: "view_layout.display_limit" })).toBe(true);
    expect(isViewRefusal({ kind: "view_layout.unsaved" })).toBe(true);
    expect(isViewRefusal({ kind: "agent_layout.display_limit" })).toBe(true);
    expect(isViewRefusal({ kind: "view_layout.stale_workspace" })).toBe(false);
    expect(isViewRefusal({ kind: "file.save_failed" })).toBe(false);
    expect(isViewRefusal(null)).toBe(false);
  });
});

describe("where a display's place and the keyboard go", () => {
  const here = "local\u0000/repo";

  it("keeps a display's place per document, so a retargeted preview starts the next one at its own place", () => {
    const a = placeKey(here, { id: "d1", tab_id: "file:/repo/a.md" });
    expect(placeKey(here, { id: "d1", tab_id: "file:/repo/b.md" })).not.toBe(a);
    expect(placeKey(here, { id: "d1", tab_id: "file:/repo/a.md" })).toBe(a);
    expect(placeKey("mini\u0000/repo", { id: "d1", tab_id: "file:/repo/a.md" })).not.toBe(a);
  });

  it("follows a moved view only on the frame that lands it, in the Workspace it was asked in", () => {
    const request = { workspace: here, displayId: "d1", from: { areaId: "a1", index: 0 } };
    const asked = layout(split("s1", "row", 0.5, area("a1", ["d1", "d2"]), area("a2", ["d3"])));
    expect(focusRequestArrived(request, here, asked)).toBe(false);
    const landed = layout(split("s1", "row", 0.5, area("a1", ["d2"]), { area: { id: "a2", active: "d1", displays: [display("d3"), display("d1")] } }));
    landed.active_area = "a2";
    expect(focusRequestArrived(request, here, landed)).toBe(true);
    expect(focusRequestArrived(request, "mini\u0000/repo", landed)).toBe(false);
    expect(focusRequestArrived({ workspace: here, areaId: "a2" }, here, asked)).toBe(false);
    expect(focusRequestArrived({ workspace: here, areaId: "a2" }, here, landed)).toBe(true);
  });

  it("follows an opened file or diff once the active area shows it past opening", () => {
    const opened = { workspace: here, opened: { path: "/repo/b.md", kind: "file" as const, committed: null } };
    const before = layout(area("a1", ["a"]));
    expect(focusRequestArrived(opened, here, before)).toBe(false);
    const opening = layout(area("a1", [display("b", { state: "opening", tab_id: null })]));
    expect(focusRequestArrived(opened, here, opening)).toBe(false);
    const shown = layout(area("a1", ["b"]));
    expect(focusRequestArrived(opened, here, shown)).toBe(true);
    expect(focusRequestArrived(opened, "mini\u0000/repo", shown)).toBe(false);
    // The same path as a diff is not the file the operator opened, nor is
    // the other History group's diff of it.
    const diff = layout(area("a1", [display("b", { kind: "diff", committed: false })]));
    expect(focusRequestArrived(opened, here, diff)).toBe(false);
    expect(focusRequestArrived({ workspace: here, opened: { path: "/repo/b.md", kind: "diff", committed: false } }, here, diff)).toBe(true);
    expect(focusRequestArrived({ workspace: here, opened: { path: "/repo/b.md", kind: "diff", committed: true } }, here, diff)).toBe(false);
    // A view that cannot show the file still takes the keyboard, so ⌘W closes it.
    const unavailable = layout(area("a1", [display("b", { state: "unavailable", tab_id: null })]));
    expect(focusRequestArrived(opened, here, unavailable)).toBe(true);
  });

  it("follows a page the operator opened once the core's answer names it and it is the active view", () => {
    const request = { workspace: here, browserOpen: "follow:1" };
    const shown = layout(area("a1", ["a", "page"]));
    shown.root = { area: { id: "a1", active: "page", displays: [display("a"), display("page", { kind: "browser", path: "" })] } };
    const answered = [{ request_id: "follow:1", ok: true, display_id: "page" }];
    expect(focusRequestArrived(request, here, shown)).toBe(false);
    expect(focusRequestArrived(request, here, shown, [{ request_id: "follow:0", ok: true, display_id: "page" }])).toBe(false);
    expect(focusRequestArrived(request, here, shown, answered)).toBe(true);
    expect(focusRequestArrived(request, "mini\u0000/repo", shown, answered)).toBe(false);
    // Not yet the active view, or refused: the keyboard stays where it is.
    const behind = layout(area("a1", ["a", "page"]));
    behind.root = { area: { id: "a1", active: "a", displays: [display("a"), display("page", { kind: "browser", path: "" })] } };
    expect(focusRequestArrived(request, here, behind, answered)).toBe(false);
    expect(focusRequestArrived(request, here, shown, [{ request_id: "follow:1", ok: false, display_id: null }])).toBe(false);
  });
});

describe("a dragged tab's target", () => {
  // a1 [d1 d2 d3] on the left (0..500), a2 [d4] on the right (502..1002).
  const tree = split("s1", "row", 0.5, area("a1", ["d1", "d2", "d3"]), area("a2", ["d4"]));
  const g = viewGeometry(tree, body(1002), SIZES);
  const tab = (displayId: string, x: number): TabSlot => ({ displayId, rect: { x, y: 0, width: 100, height: 32 } });
  const tabs = { a1: [tab("d1", 0), tab("d2", 100), tab("d3", 200)], a2: [tab("d4", 502)] };
  const at = (displayId: string, x: number, y: number, geometry = g, root = tree) => dropTarget({ layout: layout(root), geometry, sizes: SIZES, tabs, displayId, point: { x, y } });

  it("reorders within a bar by tab centres, naming the final position", () => {
    expect(at("d1", 250, 10)).toMatchObject({ kind: "bar", areaId: "a1", index: 1, line: { x: 200 } });
    expect(at("d1", 251, 10)).toMatchObject({ kind: "bar", areaId: "a1", index: 2, line: { x: 300 } });
    expect(at("d3", 40, 10)).toMatchObject({ kind: "bar", areaId: "a1", index: 0, line: { x: 0 } });
  });

  it("targets nothing where the tab already is", () => {
    expect(at("d2", 120, 10)).toEqual({ kind: "none", reason: null });
    expect(at("d2", 240, 10)).toEqual({ kind: "none", reason: null });
  });

  it("moves into another area's bar at the slot under the pointer", () => {
    expect(at("d1", 700, 10)).toMatchObject({ kind: "bar", areaId: "a2", index: 1 });
    expect(at("d1", 540, 10)).toMatchObject({ kind: "bar", areaId: "a2", index: 0 });
  });

  it("names the final position when the other area's view of the same document gives way", () => {
    // a2 [d4 t1 d5], where t1 is a second view of d1's document (Open to the side).
    const twin = display("t1", { path: "/repo/d1.md", tab_id: "file:d1", label: "d1.md" });
    const root = split("s1", "row", 0.5, area("a1", ["d1", "d2", "d3"]), area("a2", ["d4", twin, "d5"]));
    const slots = { ...tabs, a2: [tab("d4", 502), tab("t1", 602), tab("d5", 702)] };
    const drop = (x: number) => dropTarget({ layout: layout(root), geometry: g, sizes: SIZES, tabs: slots, displayId: "d1", point: { x, y: 10 } });
    expect(drop(540)).toMatchObject({ kind: "bar", areaId: "a2", index: 0, line: { x: 502 } });
    expect(drop(600)).toMatchObject({ kind: "bar", areaId: "a2", index: 1, line: { x: 602 } });
    expect(drop(700)).toMatchObject({ kind: "bar", areaId: "a2", index: 1, line: { x: 702 } });
    expect(drop(800)).toMatchObject({ kind: "bar", areaId: "a2", index: 2, line: { x: 802 } });
    // A diff of the same path in the other History group is another document.
    const branch = display("t1", { path: "/repo/d1.md", kind: "diff", committed: true, tab_id: "diff:d1" });
    const mixed = split("s1", "row", 0.5, area("a1", ["d1", "d2", "d3"]), area("a2", ["d4", branch, "d5"]));
    expect(dropTarget({ layout: layout(mixed), geometry: g, sizes: SIZES, tabs: slots, displayId: "d1", point: { x: 700, y: 10 } })).toMatchObject({ index: 2 });
  });

  it("splits at an edge only inside its zone, previewing the half the new area takes", () => {
    // a2's content is 500 wide: the right zone starts past 1002 - 0.3 * 500 = 852.
    expect(at("d1", 853, 300)).toEqual({ kind: "edge", areaId: "a2", edge: "right", preview: { x: 752, y: 0, width: 250, height: 600 }, label: "Split right" });
    expect(at("d1", 852, 300)).toMatchObject({ kind: "none", reason: expect.any(String) });
    expect(at("d1", 600, 200)).toMatchObject({ kind: "edge", edge: "left", areaId: "a2" });
  });

  it("shows no split where it cannot land and says why", () => {
    const narrow = viewGeometry(tree, body(900), SIZES);
    expect(at("d1", 890, 300, narrow)).toEqual({ kind: "none", reason: "This view area is too narrow to split." });
    expect(at("d4", 990, 300)).toEqual({ kind: "none", reason: "This is the only view in its area." });
  });

  it("targets nothing outside the Views or for a view that is gone", () => {
    expect(at("d1", 1100, 300)).toMatchObject({ kind: "none", reason: expect.any(String) });
    expect(at("gone", 700, 10)).toEqual({ kind: "none", reason: "This view is no longer open." });
  });
});

describe("area commands", () => {
  const reason = (root: ViewNode, step: ViewAreaStep, drawn = true) => {
    const views = layout(root);
    return viewAreaStepUnavailable(views, drawn ? { geometry: viewGeometry(root, body(2000), SIZES) } : null, step);
  };

  it("cannot move or resize with one area, and says so", () => {
    for (const step of ["focus_next", "focus_previous", "grow", "shrink"] as const) {
      expect(reason(area("a1", ["d1"]), step), step).toBe("There is only one view area.");
    }
  });

  it("can move and resize once there are two areas", () => {
    const two = split("s1", "row", 0.5, area("a1", ["d1"]), area("a2", ["d3"]));
    for (const step of ["focus_next", "focus_previous", "grow", "shrink"] as const) {
      expect(reason(two, step), step).toBeNull();
    }
  });

  it("grows the active area toward its sibling by one step and stops at the core's range", () => {
    const tree = (ratio: number) => split("s1", "row", ratio, area("a1", ["d1"]), area("a2", ["d2"]));
    const at = (ratio: number, activeArea: string, grow: boolean) => {
      const views = layout(tree(ratio));
      views.active_area = activeArea;
      return resizeTarget(views, null, grow);
    };
    expect(at(0.5, "a1", true)).toEqual({ splitId: "s1", ratio: 0.55 });
    expect(at(0.5, "a2", true)).toEqual({ splitId: "s1", ratio: 0.45 });
    expect(at(0.85, "a1", true)).toEqual({ reason: "This view area cannot grow any further." });
    expect(at(0.84, "a1", true)).toEqual({ splitId: "s1", ratio: 0.85 });
    expect(at(0.15, "a1", false)).toEqual({ reason: "This view area cannot shrink any further." });
  });
});

describe("a display's identity", () => {
  it("names the kind, the full path and the state", () => {
    expect(displayIdentity(display("d1", { preview: true }), t)).toBe("File: /repo/d1.md · Preview");
    expect(displayIdentity(display("d1", { state: "unavailable", reason: "No such file" }), t)).toBe("File: /repo/d1.md · Unavailable: No such file");
    expect(displayIdentity(display("d1", { kind: "diff", committed: true }), t)).toBe("Branch diff: /repo/d1.md");
    expect(displayIdentity(display("d1", { state: "waiting", reason: "Waiting for mini to connect" }), t)).toBe("File: /repo/d1.md · Waiting: Waiting for mini to connect");
  });

  it("names the kind and the state in the chosen language", () => {
    expect(displayIdentity(display("d1", { kind: "diff", committed: true, preview: true }), ko)).toBe("브랜치 변경 비교: /repo/d1.md · 미리보기");
  });
});
