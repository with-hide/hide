// @vitest-environment jsdom
// PRD overview-graph-remembered B3, B5, B13: the layout opens remembered places
// at the interface scale, and Reset layout never places the old ones again,
// neither before the core's answer nor after reopening.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { ADD_PROJECT, type OverviewGraphModel } from "./overviewGraphModel";
import { projectDropPlaces, useOverviewGraphLayout, type GraphLayout, type RememberedPlaces } from "./overviewGraphLayout";
import { ForceMap } from "./overviewMapLayout";

const TOKENS = { gap: 8, "root-width": 160, "root-height": 120, "root-orb": 48, "child-width": 120, "child-height": 96, "child-orb": 32 };
const model = {
  structure: "fixture",
  projects: [{ id: "p", shown: [{ id: "s1" }, { id: "s2" }], revealSlot: null, startSlot: "p:start" }, { id: "q", shown: [{ id: "s3" }], revealSlot: null, startSlot: "q:start" }],
  layoutParents: new Map([["s1", null], ["s2", "s1"], ["s3", null]]),
  visible: [],
} as unknown as OverviewGraphModel;
const remembered: RememberedPlaces = { anchors: new Map([["q", { x: 700, y: 500 }]]), pins: new Map([["s1", { x: -400, y: 250 }]]) };
const nothing: RememberedPlaces = { anchors: new Map(), pins: new Map() };

let layout: GraphLayout | null = null;
const host: { current: HTMLDivElement | null } = { current: null };
function Probe({ places, graph = model }: { places: RememberedPlaces; graph?: OverviewGraphModel }) {
  layout = useOverviewGraphLayout(graph, host, 0, places);
  return null;
}

/** The fixture's model with every session and project renamed by `n`, so the physics starts it from other angles. */
const renamed = (n: number) => ({
  ...model,
  structure: `fixture-${n}`,
  projects: [{ id: `p${n}`, shown: [{ id: `s1-${n}` }, { id: `s2-${n}` }], revealSlot: null, startSlot: `p${n}:start` }, { id: `q${n}`, shown: [{ id: `s3-${n}` }], revealSlot: null, startSlot: `q${n}:start` }],
  layoutParents: new Map([[`s1-${n}`, null], [`s2-${n}`, `s1-${n}`], [`s3-${n}`, null]]),
}) as unknown as OverviewGraphModel;

afterEach(() => {
  document.body.replaceChildren();
  document.documentElement.style.removeProperty("--interface-scale");
  layout = null;
  vi.unstubAllGlobals();
});

async function mount(scale: number) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  document.documentElement.style.setProperty("--interface-scale", String(scale));
  host.current = document.createElement("div");
  for (const [name, value] of Object.entries(TOKENS)) host.current.style.setProperty(`--size-overview-graph-${name}`, String(value));
  const container = document.createElement("div");
  document.body.append(host.current, container);
  return createRoot(container);
}

it("opens a remembered pin and anchor at the interface scale", async () => {
  const root = await mount(1.5);
  await act(async () => root.render(<Probe places={remembered} />));
  expect(layout!.phase).toBe("ready");
  expect(layout!.scale).toBe(1.5);
  expect(layout!.map!.pinOf("s1")).toEqual({ x: -600, y: 375 });
  expect(layout!.map!.anchorOf("q")).toEqual({ x: 1050, y: 750 });
  await act(async () => root.unmount());
});

it("does not place the reset scope's old places again before the core forgets them or after reopening", async () => {
  const root = await mount(1);
  await act(async () => root.render(<Probe places={nothing} />));
  const fresh = layout!.map!.positions();
  await act(async () => root.render(<Probe places={remembered} />));
  // The open map keeps what it placed; the remembered places are for the next opening.
  expect(layout!.map!.positions()).toEqual(fresh);
  await act(async () => root.unmount());

  const reopened = await mount(1);
  await act(async () => reopened.render(<Probe places={remembered} />));
  expect(layout!.map!.pinOf("s1")).toEqual({ x: -400, y: 250 });
  // Reset layout: the snapshot still holds the old places until the core answers.
  await act(async () => layout!.reset());
  expect(layout!.map!.pinOf("s1")).toBeUndefined();
  expect(layout!.map!.anchorOf("q")).not.toEqual({ x: 700, y: 500 });
  expect(layout!.map!.positions()).toEqual(fresh);
  // The core's answer empties them; nothing moves.
  await act(async () => reopened.render(<Probe places={nothing} />));
  expect(layout!.map!.positions()).toEqual(fresh);
  await act(async () => reopened.unmount());

  const afterReset = await mount(1);
  await act(async () => afterReset.render(<Probe places={nothing} />));
  expect(layout!.map!.positions()).toEqual(fresh);
  await act(async () => afterReset.unmount());
});

it("a project drop carries only the pins the core still remembers, at scale one", () => {
  const input = { projects: ["p"], sessions: [{ id: "s1", projectId: "p", parent: null, radius: 80 }, { id: "s2", projectId: "p", parent: null, radius: 80 }], operator: 40 };
  // This window pinned both; another window's Reset layout made the core forget s2.
  const map = new ForceMap({ ...input, remembered: { anchors: new Map([["p", { x: 300, y: 0 }]]), pins: new Map([["s1", { x: 200, y: 100 }], ["s2", { x: 400, y: 100 }]]) } });
  const dropped = projectDropPlaces(map, "p", { anchors: new Map(), pins: new Map([["s1", { x: 1, y: 1 }]]) }, 2);
  expect(dropped).toEqual({ anchor: { x: 150, y: 0 }, pins: [{ id: "s1", at: { x: 100, y: 50 } }] });
  expect(projectDropPlaces(map, "missing", nothing, 1)).toBeNull();
});

it("places each project's start circle clear of every session's circle and words, and the Add project place among the sessions (issue 2514)", async () => {
  const root = await mount(1);
  await act(async () => root.render(<Probe places={nothing} />));
  const at = layout!.map!.positions();
  // A session's room takes in its words, a little inside its neighbours' room.
  const session = Math.max(TOKENS["root-width"] / 2, TOKENS["root-height"] - TOKENS["root-orb"] / 2) * 0.85 + TOKENS.gap / 2;
  const places: [string, number][] = [["s1", session], ["s3", session], [ADD_PROJECT.slot, session]];
  for (const id of [...places.map(([id]) => id), "p:start", "q:start"]) expect(at.get(id)).toBeDefined();
  // The places keep their rooms apart, give or take the physics' last settling step.
  for (const [i, [a, ra]] of places.entries()) for (const [b, rb] of places.slice(i + 1)) expect(Math.hypot(at.get(a)!.x - at.get(b)!.x, at.get(a)!.y - at.get(b)!.y)).toBeGreaterThan((ra + rb) * 0.8);
  // A start circle moves with its project; the Add project place belongs to none.
  layout!.map!.moveProject("p", { x: 30, y: 0 });
  expect(layout!.map!.positions().get("p:start")!.x).toBeCloseTo(at.get("p:start")!.x + 30);
  expect(layout!.map!.positions().get(ADD_PROJECT.slot)).toEqual(at.get(ADD_PROJECT.slot));
  // No session's circle or words cover a start circle, so a press on it
  // reaches it, from whatever angles the physics starts the sessions at.
  const box = (kind: "root" | "child") => ({ width: TOKENS[`${kind}-width`], height: TOKENS[`${kind}-height`], orb: TOKENS[`${kind}-orb`] });
  for (let n = 0; n < 40; n += 1) {
    await act(async () => root.render(<Probe places={nothing} graph={renamed(n)} />));
    const placed = layout!.map!.positions();
    const clearance = (start: string, id: string, kind: "root" | "child") => {
      const c = placed.get(start)!, s = placed.get(id)!, b = box(kind);
      const dx = Math.max(s.x - b.width / 2 - c.x, 0, c.x - (s.x + b.width / 2));
      const dy = Math.max(s.y - b.orb / 2 - c.y, 0, c.y - (s.y + b.height - b.orb / 2));
      return Math.hypot(dx, dy) - TOKENS["root-orb"] / 2;
    };
    for (const start of [`p${n}:start`, `q${n}:start`]) for (const [id, kind] of [[`s1-${n}`, "root"], [`s2-${n}`, "child"], [`s3-${n}`, "root"]] as const) expect(clearance(start, id, kind), `${start} beside ${id}`).toBeGreaterThan(0);
  }
  await act(async () => root.unmount());
});

it("gathers the Add project place in the gap between projects, leaving every project's place as it was (issue 2514)", () => {
  const sessions = [{ id: "a1", projectId: "a", parent: null, radius: 40 }, { id: "b1", projectId: "b", parent: null, radius: 40 }];
  const without = new ForceMap({ projects: ["a", "b"], sessions, operator: 40 });
  const withFree = new ForceMap({ projects: ["a", "b", ADD_PROJECT.project], sessions: [...sessions, { id: ADD_PROJECT.slot, projectId: ADD_PROJECT.project, parent: null, radius: 40 }], operator: 40, free: ADD_PROJECT.project });
  expect(withFree.anchorOf("a")).toEqual(without.anchorOf("a"));
  expect(withFree.anchorOf("b")).toEqual(without.anchorOf("b"));
  // Two projects stand above and below the operator; the place goes to the side between them.
  expect(Math.abs(withFree.anchorOf(ADD_PROJECT.project)!.y)).toBeLessThan(1e-9);
  without.stop();
  withFree.stop();
});
