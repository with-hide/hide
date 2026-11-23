// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { GatheredList } from "./Gathered";
import { watchHost } from "./presence";
import type { PhoneMission } from "./protocol";
import { patch, usePhone } from "./store";

/** The sockets the watcher opened, in order; each is a stand-in the test speaks for. */
const sockets = vi.hoisted(() => [] as { onmessage: ((event: { data: string }) => void) | null }[]);
vi.mock("./relaySocket", () => ({
  RelaySocket: class {
    readyState = 1;
    onopen: unknown = null;
    onmessage: unknown = null;
    onclose: unknown = null;
    constructor() { sockets.push(this as never); }
    send() {}
    close() {}
  },
}));
vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

const MAC = { room: "a".repeat(43), name: "MacBook", noiseKey: "k".repeat(43), credential: "own", protocol: 5, lastSeen: null };
const MINI = { ...MAC, room: "b".repeat(43), name: "맥미니", credential: "mini" };

function mission(id: string, over: Partial<PhoneMission> = {}): PhoneMission {
  return {
    id, title: `Mission ${id}`, device_id: "local", device_label: null, status: "running", decision: null, question: null, answer: null, answer_failed: null,
    landing_order: [], created_at_unix_ms: 1_000, started_at_unix_ms: 1_000, closed_at_unix_ms: null, stale: false,
    conductor: { present: true, takes_answers: true, name: "Conductor", agent_kind: "claude", agent: null }, nodes: [], ...over,
  };
}

const initial = usePhone.getState();
const assign = vi.fn();
let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("location", { ...window.location, assign });
  sockets.length = 0;
  container = document.createElement("div");
  root = createRoot(container);
  patch({ groups: [], automationAsks: [], missionAsks: [], missions: [], listScope: "all", connected: true });
});

afterEach(async () => {
  await act(async () => root.unmount());
  usePhone.setState(initial, true);
  assign.mockReset();
  vi.unstubAllGlobals();
});

// One number (docs/UI_BEHAVIOR.md, the phone's 모두): a mission that is its own 내 차례 item on another computer counts and stands in
// 모두's 내 차례 as that computer counts it, with its mark, and opens on that computer.
it("gathers another computer's missions that are their own 내 차례 item from its missions frame", async () => {
  await act(async () => root.render(<GatheredList hosts={[MAC, MINI]} activeRoom={MAC.room} dim={false} watch={watchHost} />));
  expect(sockets).toHaveLength(1);
  const send = async (frame: unknown) => act(async () => sockets[0]!.onmessage!({ data: JSON.stringify(frame) }));
  const merge = mission("m-merge", { title: "Merge permission", decision: "Merge?" });
  await send({ type: "hello" });
  // Before its list, the computer is still connecting: its missions wait for the list rather than stand alone.
  await send({ type: "missions", failed: false, items: [merge, mission("m-run")], asks: ["m-merge"] });
  expect(container.querySelector('[data-phone-group="needs_you"]')).toBeNull();
  await send({ type: "agents", groups: [], devices: [] });

  const needs = container.querySelector('[data-phone-group="needs_you"]')!;
  expect(needs.querySelector("[data-phone-group-count]")!.textContent).toBe("1");
  const row = container.querySelector<HTMLElement>(`[data-phone-gathered-mission="${MINI.room}|m-merge"]`)!;
  expect(row.textContent).toContain("Merge permission");
  expect(row.querySelector("[data-phone-computer]")?.getAttribute("data-phone-computer")).toBe("맥미니");
  expect(container.querySelectorAll("[data-phone-mission]")).toHaveLength(1);
  await act(async () => row.querySelector<HTMLElement>("[data-phone-mission]")!.click());
  expect(assign).toHaveBeenCalledWith(`/?host=${MINI.room}&mission=m-merge`);

  // Resolved there, it leaves here.
  await send({ type: "missions", failed: false, items: [merge, mission("m-run")], asks: [] });
  expect(container.querySelector('[data-phone-group="needs_you"]')).toBeNull();
});
