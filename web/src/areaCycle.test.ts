import { emptyScope } from "../test/legacyAgentScope";
import { beforeEach, describe, expect, it } from "vitest";
import { agentOrigin, areaCycle, focusedCycleScope, placeCycle, placeOrigin } from "./areaCycle";
import { noteAreaFrame } from "./areaFrames";
import { areaGeometry } from "./areaLayout";
import { createActions } from "./actions";
import { commitCycle, deskReturn, reconcileHeldCycle } from "./keyboard";
import { configurePlaceVisits, expectPlace, observeEntries, observePlace, paneKey, resetRecent, tabSurface } from "./recent";
import type { RecentPlace, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { AUTOMATION_ENTRY, MISSION_ENTRY, useUiStore } from "./ui";
import type { KeyboardOwner } from "./viewFocus";

// a1 exists independently in Agent and View; another Agent area and
// checkout both contain tabs that must never leak into the cycle.
function world(): SnapshotRest {
  const limits = { areas: 6, depth: 5, displays: 64 };
  const tabs = ["t1", "t2", "t3", "outside"].map((id) => ({ id, label: id, panes: [{ id: `${id}-pane` }], workspace_id: "w", checkout_id: "c", empty: false, delegated: false }));
  const checkout = { agent_scope: emptyScope(), id: "c", workspace_id: "w", path: "/fixture", label: "fixture", tabs, active_tab_id: "t1", strip: [] };
  return {
    navigator: { focused_device_id: "local", focused_checkout_id: "c", focused_workspace_id: "w", devices: [{ id: "local", label: "This Mac", kind: "local", state: "local" }], workspaces: [{ agent_scope: emptyScope(), id: "w", label: "fixture", device_id: "local", checkouts: [checkout, { ...checkout, id: "other", path: "/other", tabs: [{ ...tabs[0], id: "other-tab" }] }] }] },
    workspace_view: { device_id: "local", path: "/fixture", open: true, tray: { tab_id: "t1", shown: true, count: 0 }, tool: null, layout: { root: { area: { id: "a1", active: "d1", displays: ["d1", "d2"].map((id) => ({ id, kind: "browser", label: id, url: "about:blank", state: "open", tab_id: null })) } }, active_area: "a1", display_count: 2, limits }, agent_layout: { root: { split: { id: "s1", axis: "row", ratio: 0.5, first: { area: { id: "a1", active: "t1", displays: ["t1", "t2", "t3"].map((id) => ({ id })) } }, second: { area: { id: "a2", active: "outside", displays: [{ id: "outside" }] } } } }, active_area: "a1", canvases: {}, display_count: 4, limits } },
  } as unknown as SnapshotRest;
}
function draw(rest: SnapshotRest) {
  const view = rest.workspace_view!;
  const sizes = { areaMinWidth: 100, areaMinHeight: 100, divider: 4, tabStrip: 30 };
  for (const kind of ["agent", "view"] as const) {
    const layout = kind === "agent" ? view.agent_layout! : view.layout!;
    noteAreaFrame(kind, { workspace: { device_id: "local", path: "/fixture" }, layout, geometry: areaGeometry(layout.root, { x: 0, y: 0, width: 1200, height: 800 }, sizes), sizes } as never);
  }
  useShellStore.setState({ rest });
}
const ids = (cycle: ReturnType<typeof areaCycle>) => cycle?.items.map((row) => row.target.kind === "surface" ? row.target.surface.id : "wrong-kind");
const owner = { kind: "pane", workspace: "c", paneId: "t1-pane" } as const;

describe("focused-area recent tabs", () => {
  beforeEach(() => { resetRecent(); useUiStore.setState({ screen: { kind: "workspace" }, cycle: null }); noteAreaFrame("agent", null); noteAreaFrame("view", null); });
  it("narrows a View area to its own tabs despite an Agent area with the same ID", () => {
    const rest = world(); draw(rest);
    const checkout = rest.navigator!.workspaces![0]!.checkouts[0]!;
    observeEntries(rest, tabSurface(checkout, "t2", "local"));
    expect(ids(areaCycle(rest, { kind: "view", workspace: "c", areaId: "a1" }))).toEqual(["d1", "d2"]);
    // The Agent area walks panes instead (see the Agent pane cycle below).
    expect(areaCycle(rest, owner)).toBeNull();
    expect(areaCycle(rest, { kind: "agent", workspace: "c", areaId: "a1" })).toBeNull();
    expect(placeCycle(rest, { kind: "view", workspace: "c", areaId: "a1" })).toBeNull();
  });
  it("commits exactly one in-place View selection and never dispatches a preview", () => {
    const rest = world(); draw(rest);
    const sent: unknown[] = [];
    const actions = createActions((event) => { sent.push(event); });
    const views = areaCycle(rest, { kind: "view", workspace: "c", areaId: "a1" })!;
    commitCycle(views, actions);
    expect(sent).toEqual([]);
    commitCycle({ ...views, index: 1 }, actions);
    expect(sent).toEqual([expect.objectContaining({ kind: "view_layout", payload: expect.objectContaining({ action: "focus", display_id: "d2" }) })]);
    expect(useUiStore.getState().viewFocusRequest).toEqual({ workspace: "local\u0000/fixture", displayId: "d2", from: null });
  });
  it("prunes a moved View tab using the new snapshot, keeps the frozen order and cancels a removed origin", () => {
    const rest = world(); draw(rest);
    const viewOwner = { kind: "view", workspace: "c", areaId: "a1" } as const;
    const area = (displays: string[], id = "a1") => ({ area: { id, active: "d1", displays: displays.map((name) => ({ id: name, kind: "browser", label: name, url: "about:blank", state: "open", tab_id: null })) } });
    rest.workspace_view!.layout!.root = area(["d1", "d2", "d3"]) as never;
    draw(rest);
    const held = { ...areaCycle(rest, viewOwner)!, index: 1 };
    rest.workspace_view!.layout!.root = area(["d1", "d3"]) as never;
    expect(ids(reconcileHeldCycle(held, rest))).toEqual(["d1", "d3"]);
    expect(reconcileHeldCycle(held, rest)?.index).toBe(1);
    rest.workspace_view!.layout!.root = area(["d1", "d3"], "new-area") as never;
    expect(reconcileHeldCycle(held, rest)).toBeNull();
  });
  it("keeps View scope strict for tools, another checkout, hidden Views and delegated canvases", () => {
    const rest = world(); draw(rest);
    observePlace(rest, pane("t2-pane"));
    expect(areaCycle(rest, { kind: "tool", workspace: "c" })).toBeNull();
    expect(areaCycle(rest, { kind: "view", workspace: "other", areaId: "a1" })).toBeNull();
    useUiStore.setState({ screen: { kind: "main" } });
    expect(areaCycle(rest, owner)).toBeNull();
    useUiStore.setState({ screen: { kind: "workspace" } });
    // The desk folded behind a tool: its Views are out of sight (PRD lane-two-columns B11).
    rest.workspace_view!.tray.shown = false;
    rest.workspace_view!.tool = "changes";
    expect(areaCycle(rest, { kind: "view", workspace: "c", areaId: "a1" })).toBeNull();
    rest.workspace_view!.agent_layout!.canvases.a1 = "outside";
    expect(focusedCycleScope(rest, { kind: "pane", workspace: "c", paneId: "outside-pane" })?.areaId).toBe("a2");
    expect(focusedCycleScope(rest, owner)).toBeNull();
  });
});

// Two devices, three checkouts: fixture (c) and other on this Mac, and a
// checkout on the mini. Every pane runs an agent but two plain shells: t1's
// second pane (a dev server) and t3's.
function devices(): SnapshotRest {
  const rest = world();
  const nav = rest.navigator!;
  const fixture = nav.workspaces![0]!.checkouts[0]!;
  fixture.tabs[0]!.panes = [{ id: "t1-pane" }, { id: "t1-side", terminal_title: "vite" }] as never;
  nav.workspaces![0]!.checkouts[1]!.tabs[0]!.panes = [{ id: "other-tab-pane" }] as never;
  nav.workspaces![0]!.checkouts[1]!.label = "review";
  nav.devices!.push({ id: "mini", label: "mini", kind: "remote", state: "connected" } as never);
  const agent = (pane: string, label: string) => ({ id: label, pane_id: pane, identity_label: label, agent_kind: "claude", mark: "going", status_code: "working" });
  nav.agents = [agent("t1-pane", "planner"), agent("t2-pane", "writer"), agent("other-tab-pane", "fixer"), agent("outside-pane", "porter"), agent("child-pane", "child")] as never;
  const miniTab = { id: "remote:mini:tab:m1", label: "m1", panes: [{ id: "remote:mini:pane:p1", device_id: "mini" }], workspace_id: "remote:mini:workspace:w", checkout_id: "remote:mini:checkout:c", empty: false, delegated: false };
  const miniCheckout = { agent_scope: emptyScope(), id: "remote:mini:checkout:c", workspace_id: "remote:mini:workspace:w", path: "/mini", label: "api", tabs: [miniTab], active_tab_id: miniTab.id, strip: [] };
  rest.status = { remote: [{ target_id: "mini", state: "connected", session: { workspaces: [{ agent_scope: emptyScope(), id: "remote:mini:workspace:w", label: "api", device_id: "mini", checkouts: [miniCheckout] }], agents: [{ id: "b", pane_id: "remote:mini:pane:p1", identity_label: "reviewer", agent_kind: "codex", mark: "going", status_code: "working" }], active_tab_ids: {}, focused_workspace_id: null, focused_checkout_id: null, focused_tab_id: null, focused_pane_id: null, pane_layouts: [] } }] } as never;
  return rest;
}
const panes = (cycle: ReturnType<typeof placeCycle>) => cycle?.items.map((row) => row.target.kind === "pane" ? row.target.paneId : "wrong-kind");
const pane = (id: string): RecentPlace => ({ kind: "pane", pane_id: id });
const panes_ = (...ids: string[]): RecentPlace[] => ids.map(pane);
/** The devices fixture with each reported visit echoed into its `ui_state` as the core keeps it: moved to the front, once. */
function visited(): SnapshotRest {
  const rest = devices();
  configurePlaceVisits((place) => {
    const held = rest.ui_state?.recent_places ?? [];
    rest.ui_state = { ...rest.ui_state, recent_places: [place, ...held.filter((row) => JSON.stringify(row) !== JSON.stringify(place))] };
  });
  return rest;
}

describe("recent place cycle: agent panes (issue 301)", () => {
  beforeEach(() => { resetRecent(); useUiStore.setState({ screen: { kind: "workspace" }, cycle: null }); noteAreaFrame("agent", null); noteAreaFrame("view", null); });

  it.each<KeyboardOwner>([{ kind: "none" }, { kind: "tool", workspace: "c" }])("opens recent agents from $kind focus without inventing a pane visit", (outside) => {
    const rest = visited(); draw(rest);
    for (const id of ["remote:mini:pane:p1", "t2-pane", "other-tab-pane"]) observePlace(rest, pane(id));
    expect(agentOrigin(rest, outside)).toBeNull();
    const cycle = placeCycle(rest, outside)!;
    expect(panes(cycle)).toEqual(["other-tab-pane", "t2-pane", "remote:mini:pane:p1"]);
    expect(cycle.index).toBe(-1);
    expect(cycle.originKey).toBeUndefined();
    const sent: unknown[] = [];
    commitCycle({ ...cycle, index: 0 }, createActions((event) => { sent.push(event); }));
    expect(sent).toEqual([expect.objectContaining({ kind: "focus_pane", payload: expect.objectContaining({ pane_id: "other-tab-pane" }) })]);
    expect(panes(placeCycle(rest, outside))).toEqual(["other-tab-pane", "t2-pane", "remote:mini:pane:p1"]);
  });

  it("starts at the pane the page just reported, before the core echoes it, then follows the core", () => {
    const rest = devices(); draw(rest);
    rest.ui_state = { ...rest.ui_state, recent_places: panes_("t2-pane", "other-tab-pane") };
    const reported: string[] = [];
    configurePlaceVisits((place) => { reported.push(place.kind === "pane" ? place.pane_id : place.kind); });
    const outside = { kind: "none" } as const;
    observePlace(rest, pane("other-tab-pane"));
    expect(reported).toEqual(["other-tab-pane"]);
    // The echo is still on the socket: the core's order has t2-pane first.
    expect(panes(placeCycle(rest, outside))).toEqual(["other-tab-pane", "t2-pane"]);
    rest.ui_state = { ...rest.ui_state, recent_places: panes_("other-tab-pane", "t2-pane") };
    observePlace(rest, null);
    expect(panes(placeCycle(rest, outside))).toEqual(["other-tab-pane", "t2-pane"]);
    // Another window's visit moves the core's head afterwards: the core's order stands.
    rest.ui_state = { ...rest.ui_state, recent_places: panes_("t2-pane", "other-tab-pane") };
    observePlace(rest, null);
    expect(panes(placeCycle(rest, outside))).toEqual(["t2-pane", "other-tab-pane"]);
    expect(reported).toEqual(["other-tab-pane"]);
  });

  it("opens recent agents on Main and Overview even with no drawn Agent area", () => {
    const rest = visited(); draw(rest);
    observePlace(rest, pane("other-tab-pane"));
    noteAreaFrame("agent", null);
    for (const screen of [{ kind: "main" }] as const) {
      useUiStore.setState({ screen });
      // A shortcut can leave the last View owner recorded when its screen
      // unmounts; that is no longer a focused file on Main or Overview.
      for (const outside of [{ kind: "none" }, { kind: "view", workspace: "c", areaId: "a1" }] as const) {
        expect(panes(placeCycle(rest, outside))).toEqual(["other-tab-pane"]);
        expect(placeCycle(rest, outside)?.index).toBe(-1);
      }
    }
    rest.ui_state = { ...rest.ui_state, recent_places: [] };
    expect(placeCycle(rest, { kind: "none" })).toBeNull();
  });

  it("never substitutes Agent panes for a focused View, including a single-tab or retired area", () => {
    const rest = visited(); draw(rest);
    observePlace(rest, pane("other-tab-pane"));
    const viewOwner = { kind: "view", workspace: "c", areaId: "a1" } as const;
    expect(placeCycle(rest, viewOwner)).toBeNull();
    expect(ids(areaCycle(rest, viewOwner))).toEqual(["d1", "d2"]);
    rest.workspace_view!.layout!.root = { area: { id: "a1", active: "d1", displays: [{ id: "d1", kind: "browser", label: "d1", url: "about:blank", state: "open", tab_id: null }] } } as never;
    expect(areaCycle(rest, viewOwner)).toBeNull();
    expect(placeCycle(rest, viewOwner)).toBeNull();
    rest.workspace_view!.tool = "changes";
    expect(placeCycle(rest, viewOwner)).toBeNull();
  });

  it("walks the agent panes visited on every device, project and checkout, one row per pane, most recent first", () => {
    const rest = visited(); draw(rest);
    for (const id of ["remote:mini:pane:p1", "other-tab-pane", "t1-side", "t2-pane", "t1-pane"]) observePlace(rest, pane(id));
    const cycle = placeCycle(rest, owner)!;
    expect(cycle.kind).toBe("places");
    // t1-side was visited but runs no agent; outside was never visited; no View display is a row.
    expect(panes(cycle)).toEqual(["t1-pane", "t2-pane", "other-tab-pane", "remote:mini:pane:p1"]);
    expect(cycle.index).toBe(0);
    const rows = Object.fromEntries(cycle.items.map((row) => [row.target.kind === "pane" ? row.target.paneId : row.key, row]));
    expect(rows["t1-pane"]).toMatchObject({ title: "planner", detail: { kind: "surface", place: "fixture", surface: "engine" }, chip: null, agent: expect.objectContaining({ agent_kind: "claude" }) });
    expect(rows["other-tab-pane"]).toMatchObject({ title: "fixer", detail: { kind: "surface", place: "fixture · review", surface: "engine" }, chip: null });
    expect(rows["remote:mini:pane:p1"]).toMatchObject({ title: "reviewer", detail: { kind: "surface", place: "api", surface: "engine" }, chip: { label: "mini", local: false }, agent: expect.objectContaining({ agent_kind: "codex" }) });
  });

  it("from a pane with no agent, lands first on the most recent agent pane", () => {
    const rest = visited(); draw(rest);
    for (const id of ["t2-pane", "other-tab-pane", "t1-side"]) observePlace(rest, pane(id));
    const shell = { kind: "pane", workspace: "c", paneId: "t1-side" } as const;
    expect(agentOrigin(rest, shell)).toEqual({ paneId: "t1-side" });
    const cycle = placeCycle(rest, shell)!;
    expect(panes(cycle)).toEqual(["other-tab-pane", "t2-pane"]);
    expect(cycle.index).toBe(-1);
    const sent: { kind: string; payload: Record<string, unknown> }[] = [];
    commitCycle({ ...cycle, index: 0 }, createActions((event) => { sent.push(event as never); }));
    expect(sent).toEqual([expect.objectContaining({ kind: "focus_pane", payload: expect.objectContaining({ pane_id: "other-tab-pane" }) })]);
  });

  it("commits one event naming the pane, through the device it is on, and nothing for the origin or a preview", () => {
    const rest = visited(); draw(rest);
    for (const id of ["remote:mini:pane:p1", "other-tab-pane", "t1-pane"]) observePlace(rest, pane(id));
    const sent: { kind: string; payload: Record<string, unknown> }[] = [];
    const actions = createActions((event) => { sent.push(event as never); });
    const cycle = placeCycle(rest, owner)!;
    expect(commitCycle(cycle, actions)).toBe(false);
    expect(sent).toEqual([]);
    commitCycle({ ...cycle, index: 1 }, actions);
    expect(sent).toEqual([expect.objectContaining({ kind: "focus_pane", payload: expect.objectContaining({ pane_id: "other-tab-pane", focus_device: false }) })]);
    sent.length = 0;
    commitCycle({ ...cycle, index: 2 }, actions);
    expect(sent).toEqual([expect.objectContaining({ kind: "remote_control", payload: expect.objectContaining({ target_id: "mini", action: "focus_pane", pane_id: "remote:mini:pane:p1", focus_device: true }) })]);
  });

  it("keeps the one other row when the origin's own agent ends while held", () => {
    const rest = visited(); draw(rest);
    for (const id of ["t2-pane", "t1-pane"]) observePlace(rest, pane(id));
    const held = { ...placeCycle(rest, owner)!, index: 1 };
    expect(panes(held)).toEqual(["t1-pane", "t2-pane"]);
    rest.navigator!.agents = rest.navigator!.agents!.filter((row) => row.pane_id !== "t1-pane");
    const kept = reconcileHeldCycle(held, rest);
    expect(panes(kept)).toEqual(["t2-pane"]);
    expect(kept?.index).toBe(0);
    const sent: { kind: string; payload: Record<string, unknown> }[] = [];
    commitCycle(kept!, createActions((event) => { sent.push(event as never); }));
    expect(sent).toEqual([expect.objectContaining({ kind: "focus_pane", payload: expect.objectContaining({ pane_id: "t2-pane" }) })]);
  });

  it("reports a visit to the core once it is not first there, and never a pane no tab holds", () => {
    const rest = devices(); draw(rest);
    const sent: string[] = [];
    const core = (...ids: string[]) => { rest.ui_state = { ...rest.ui_state, recent_places: panes_(...ids) }; };
    configurePlaceVisits((place) => { sent.push(place.kind === "pane" ? place.pane_id : place.kind); });
    observePlace(rest, pane("t2-pane"));
    core("t2-pane");
    observePlace(rest, pane("t2-pane"));
    expect(sent).toEqual(["t2-pane"]);
    // Back to t2 before the core echoed t1 is still a visit; a closed pane is none.
    observePlace(rest, pane("t1-pane"));
    observePlace(rest, pane("t2-pane"));
    observePlace(rest, pane("gone-pane"));
    expect(sent).toEqual(["t2-pane", "t1-pane", "t2-pane"]);
    // Another window's visit reached the core last, so the same pane is reported again.
    core("other-tab-pane", "t2-pane", "t1-pane");
    observePlace(rest, pane("t2-pane"));
    expect(sent).toEqual(["t2-pane", "t1-pane", "t2-pane", "t2-pane"]);
  });

  it("does not take a commit's passing frames for visits", () => {
    const rest = visited(); draw(rest);
    for (const id of ["t2-pane", "outside-pane"]) observePlace(rest, pane(id));
    expectPlace(paneKey("other-tab-pane"));
    observePlace(rest, pane("t3-pane"));
    observePlace(rest, pane("other-tab-pane"));
    observePlace(rest, pane("t1-pane"));
    expect(panes(placeCycle(rest, owner))).toEqual(["t1-pane", "other-tab-pane", "outside-pane", "t2-pane"]);
  });

  it("takes no origin from a pane whose tab no Agent area draws", () => {
    const rest = visited(); draw(rest);
    observePlace(rest, pane("t2-pane"));
    // t3 is in the fixture but a1 shows t1: a keyboard owner left on t3's pane is stale.
    expect(agentOrigin(rest, { kind: "pane", workspace: "c", paneId: "t3-pane" })).toBeNull();
    expect(panes(placeCycle(rest, { kind: "pane", workspace: "c", paneId: "t3-pane" }))).toEqual(["t2-pane"]);
  });

  it("starts before the most recent agent pane when the keyboard is in no pane, and drops a closed pane or ended agent while held", () => {
    const rest = visited(); draw(rest);
    for (const id of ["t2-pane", "outside-pane", "other-tab-pane"]) observePlace(rest, pane(id));
    const bar = { kind: "agent", workspace: "c", areaId: "a1" } as const;
    // The core reports no focused pane, so the tab bar is in no pane.
    expect(agentOrigin(rest, bar)).toEqual({ paneId: null });
    const cycle = placeCycle(rest, bar)!;
    expect(panes(cycle)).toEqual(["other-tab-pane", "outside-pane", "t2-pane"]);
    expect(cycle.index).toBe(-1);
    rest.focused = { pane_id: "t1-side" };
    expect(agentOrigin(rest, bar)).toEqual({ paneId: "t1-side" });
    const held = { ...cycle, index: 0 };
    const fixture = rest.navigator!.workspaces![0]!.checkouts[0]!;
    fixture.tabs = fixture.tabs.filter((tab) => tab.id !== "t2");
    rest.navigator!.agents = rest.navigator!.agents!.filter((row) => row.pane_id !== "outside-pane");
    expect(panes(reconcileHeldCycle(held, rest))).toEqual(["other-tab-pane"]);
    rest.navigator!.agents = [];
    expect(reconcileHeldCycle(held, rest)).toBeNull();
  });

  it("counts a delegated child's canvas as the Agent area", () => {
    const rest = visited();
    const checkout = rest.navigator!.workspaces![0]!.checkouts[0]!;
    // A delegated child's tab is in no area's strip; a1 draws it as a canvas over t1.
    checkout.tabs.push({ id: "child", label: "child", panes: [{ id: "child-pane" }], workspace_id: "w", checkout_id: "c", empty: false, delegated: true } as never);
    rest.workspace_view!.agent_layout!.canvases.a1 = "child";
    draw(rest);
    observePlace(rest, pane("t2-pane"));
    const child = { kind: "pane", workspace: "c", paneId: "child-pane" } as const;
    expect(panes(placeCycle(rest, child))).toEqual(["child-pane", "t2-pane"]);
    // t1 is under the canvas now, so its pane is not where the keyboard is.
    expect(agentOrigin(rest, owner)).toBeNull();
  });
});

// The devices fixture with a mission of this Mac, one of the mini, one an
// external tool keeps, an automation with two runs not looked at, and Desk
// switched on.
function places(): SnapshotRest {
  const rest = visited();
  const mission = (id: string, title: string, device: string, workspace: string, extra: object = {}) => ({ id, title, device_id: device, workspace_id: workspace, project: `/${workspace}`, checkout_path: `/${workspace}`, status: "running", nodes: [], ...extra });
  rest.missions = { items: [
    mission("m-here", "payments refactor", "local", "w"),
    mission("m-mini", "remote release", "mini", "remote:mini:workspace:w", { status: "merge_wait" }),
    mission("m-tool", "tool's review", "local", "w", { tool: { connection: "conn-1", owns: true } }),
  ] } as never;
  useShellStore.setState({
    automations: { items: [{ id: "auto-1", name: "morning issues", unseen: 2 }], unseen: 2 } as never,
    desk: { settings: { enabled: true } } as never,
    deskJobs: { jobs: [] } as never,
  });
  return rest;
}
const mission = (id: string): RecentPlace => ({ kind: "mission", mission_id: id });
const automation = (id: string): RecentPlace => ({ kind: "automation", automation_id: id });
const desk: RecentPlace = { kind: "desk" };
const keys = (cycle: ReturnType<typeof placeCycle>) => cycle?.items.map((row) => row.key.replace("\u0000", ":"));

describe("recent place cycle: missions, automations and Desk", () => {
  beforeEach(() => {
    resetRecent();
    useUiStore.setState({ screen: { kind: "workspace" }, overviewOpen: false, cycle: null, missions: MISSION_ENTRY, automations: AUTOMATION_ENTRY });
    useShellStore.setState({ automations: null, desk: null, deskJobs: null });
    noteAreaFrame("agent", null);
    noteAreaFrame("view", null);
  });

  it("names the place on screen: a pane, a mission's or an automation's detail, Desk, and no list or form", () => {
    const rest = places(); draw(rest);
    expect(placeOrigin(rest, owner)).toEqual(pane("t1-pane"));
    useUiStore.setState({ screen: { kind: "missions" } });
    expect(placeOrigin(rest, owner)).toBeNull();
    useUiStore.getState().setMissions({ selected: "m-here" });
    expect(placeOrigin(rest, owner)).toEqual(mission("m-here"));
    useUiStore.setState({ screen: { kind: "automations" } });
    expect(placeOrigin(rest, owner)).toBeNull();
    useUiStore.getState().setAutomations({ selected: "auto-1" });
    expect(placeOrigin(rest, owner)).toEqual(automation("auto-1"));
    useUiStore.getState().setAutomations({ form: "edit" });
    expect(placeOrigin(rest, owner)).toBeNull();
    useUiStore.setState({ screen: { kind: "desk" } });
    expect(placeOrigin(rest, owner)).toEqual(desk);
    useUiStore.setState({ overviewOpen: true });
    expect(placeOrigin(rest, owner)).toBeNull();
    useUiStore.setState({ overviewOpen: false, screen: { kind: "main" } });
    expect(placeOrigin(rest, owner)).toBeNull();
  });

  it("walks panes, missions, an automation and Desk in one order, each row drawn from what the core lists", () => {
    const rest = places(); draw(rest);
    for (const place of [pane("other-tab-pane"), mission("m-mini"), automation("auto-1"), desk, mission("m-here"), pane("t1-pane")]) observePlace(rest, place);
    const cycle = placeCycle(rest, owner)!;
    expect(keys(cycle)).toEqual(["pane:t1-pane", "mission:m-here", "desk", "automation:auto-1", "mission:m-mini", "pane:other-tab-pane"]);
    expect(cycle.index).toBe(0);
    const rows = Object.fromEntries(cycle.items.map((row) => [row.key.replace("\u0000", ":"), row]));
    expect(rows["mission:m-here"]).toMatchObject({ title: "payments refactor", kind: "mission", detail: { kind: "mission", project: "fixture", status: "running" }, chip: null, target: { kind: "mission", missionId: "m-here" } });
    expect(rows["mission:m-mini"]).toMatchObject({ title: "remote release", detail: { kind: "mission", project: "api", status: "merge_wait" }, chip: { label: "mini", local: false } });
    expect(rows["automation:auto-1"]).toMatchObject({ title: "morning issues", kind: "automation", detail: { kind: "automation", unseen: 2 }, chip: null, target: { kind: "automation", automationId: "auto-1" } });
    expect(rows["desk"]).toMatchObject({ kind: "desk", detail: { kind: "desk", asking: 0 }, target: { kind: "desk" } });
    // A mission or automation the core no longer lists, and Desk switched off, are no rows.
    rest.missions = { items: rest.missions!.items.filter((row) => row.id !== "m-mini") } as never;
    useShellStore.setState({ automations: { items: [], unseen: 0 } as never, desk: { settings: { enabled: false } } as never });
    expect(keys(placeCycle(rest, owner))).toEqual(["pane:t1-pane", "mission:m-here", "pane:other-tab-pane"]);
  });

  it("from a mission's detail, starts on it and lands first on the place before it; from a list, on the most recent place", () => {
    const rest = places(); draw(rest);
    for (const place of [automation("auto-1"), pane("t2-pane"), mission("m-here")]) observePlace(rest, place);
    useUiStore.setState({ screen: { kind: "missions" } });
    useUiStore.getState().setMissions({ selected: "m-here" });
    const cycle = placeCycle(rest, { kind: "none" })!;
    expect(keys(cycle)).toEqual(["mission:m-here", "pane:t2-pane", "automation:auto-1"]);
    expect(cycle.index).toBe(0);
    expect(cycle.originKey).toBe("mission\u0000m-here");
    useUiStore.getState().setMissions({ selected: null });
    const fromList = placeCycle(rest, { kind: "none" })!;
    expect(keys(fromList)).toEqual(["mission:m-here", "pane:t2-pane", "automation:auto-1"]);
    expect(fromList.index).toBe(-1);
  });

  it("commits a mission, an automation or Desk on the page's own screen, closing the Overview over it, with no event to the core", () => {
    const rest = places(); draw(rest);
    for (const place of [desk, automation("auto-1"), mission("m-mini"), mission("m-tool"), pane("t1-pane")]) observePlace(rest, place);
    const sent: { kind: string; payload: Record<string, unknown> }[] = [];
    const actions = createActions((event) => { sent.push(event as never); });
    const row = (key: string) => {
      const cycle = placeCycle(rest, owner)!;
      return { ...cycle, index: cycle.items.findIndex((item) => item.key === key) };
    };
    useUiStore.setState({ overviewOpen: true });
    expect(commitCycle(row("mission\u0000m-mini"), actions)).toBe(true);
    expect(useUiStore.getState()).toMatchObject({ overviewOpen: false, screen: { kind: "missions" }, missions: { deviceId: "mini", selected: "m-mini" } });
    // A tool's mission is listed only in its tool's cell, so it opens there.
    expect(commitCycle(row("mission\u0000m-tool"), actions)).toBe(true);
    expect(useUiStore.getState()).toMatchObject({ screen: { kind: "missions", tool: "conn-1" }, missions: { selected: "m-tool" } });
    expect(commitCycle(row("automation\u0000auto-1"), actions)).toBe(true);
    expect(useUiStore.getState()).toMatchObject({ screen: { kind: "automations" }, automations: { selected: "auto-1", form: null } });
    useUiStore.setState({ overviewOpen: true });
    expect(commitCycle(row("desk"), actions)).toBe(true);
    expect(useUiStore.getState()).toMatchObject({ overviewOpen: false, screen: { kind: "desk" } });
    // Only the device move of the mini's mission reached the core.
    expect(sent.map((event) => event.kind).filter((kind) => kind !== "focus_device")).toEqual([]);
  });

  it("reports a mission, an automation or Desk only while the core lists it, and waits for a commit's place", () => {
    const rest = places(); draw(rest);
    const reported: RecentPlace[] = [];
    configurePlaceVisits((place) => { reported.push(place); });
    observePlace(rest, mission("gone"));
    observePlace(rest, automation("gone"));
    useShellStore.setState({ desk: { settings: { enabled: false } } as never });
    observePlace(rest, desk);
    expect(reported).toEqual([]);
    useShellStore.setState({ desk: { settings: { enabled: true } } as never });
    expectPlace("automation\u0000auto-1");
    observePlace(rest, mission("m-here"));
    observePlace(rest, automation("auto-1"));
    observePlace(rest, desk);
    expect(reported).toEqual([automation("auto-1"), desk]);
  });

  it("leaves Desk for a mission used just before it, where the first chord from Desk lands, and otherwise for the tab used before it", () => {
    const rest = places(); draw(rest);
    const checkout = rest.navigator!.workspaces![0]!.checkouts[0]!;
    observeEntries(rest, tabSurface(checkout, "t3", "local"));
    for (const place of [pane("t2-pane"), mission("m-here"), desk]) observePlace(rest, place);
    useUiStore.setState({ screen: { kind: "desk" } });
    const cycle = placeCycle(rest, { kind: "none" })!;
    expect(keys(cycle)).toEqual(["desk", "mission:m-here", "pane:t2-pane"]);
    expect(cycle.index).toBe(0);
    expect(deskReturn(rest)?.key).toBe(cycle.items[1]!.key);
    // A mission that is no row now gives way to the tab used before Desk, as it does after a pane, one running no agent too.
    rest.missions = { items: [] } as never;
    expect(deskReturn(rest)?.key).toBe(tabSurface(checkout, "t3", "local").key);
  });
});
