// @vitest-environment jsdom
// PRD overview-graph-remembered B3, B5, B13: the layout opens remembered places
// at the interface scale, and Reset layout never places the old ones again,
// neither before the core's answer nor after reopening.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { OverviewGraphModel } from "./overviewGraphModel";
import { projectDropPlaces, useOverviewGraphLayout, type GraphLayout, type RememberedPlaces } from "./overviewGraphLayout";
import { ForceMap } from "./overviewMapLayout";

const TOKENS = { gap: 8, "root-width": 160, "root-height": 120, "root-orb": 48, "child-width": 120, "child-height": 96, "child-orb": 32 };
const model = {
  structure: "fixture",
  projects: [{ id: "p", shown: [{ id: "s1" }, { id: "s2" }], revealSlot: null }, { id: "q", shown: [{ id: "s3" }], revealSlot: null }],
  layoutParents: new Map([["s1", null], ["s2", "s1"], ["s3", null]]),
  visible: [],
} as unknown as OverviewGraphModel;
const remembered: RememberedPlaces = { anchors: new Map([["q", { x: 700, y: 500 }]]), pins: new Map([["s1", { x: -400, y: 250 }]]) };
const nothing: RememberedPlaces = { anchors: new Map(), pins: new Map() };

let layout: GraphLayout | null = null;
const host: { current: HTMLDivElement | null } = { current: null };
function Probe({ places }: { places: RememberedPlaces }) {
  layout = useOverviewGraphLayout(model, host, 0, places);
  return null;
}

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
