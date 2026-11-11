// @vitest-environment jsdom
// A mission's review gates on the phone (PRD mission-human-gate B13, B18,
// B21, B22): the same strip, plan list, list-row count and wording as the
// desktop, from the gate the Mac sends with each step.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import type { PhoneMission, PhoneMissionNode, ServerFrame } from "./protocol";

vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

class FakeSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static instances: FakeSocket[] = [];
  readyState = 1;
  sent: Record<string, unknown>[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  constructor() {
    FakeSocket.instances.push(this);
  }
  send(data: string) {
    this.sent.push(JSON.parse(data) as Record<string, unknown>);
  }
  close() {}
}

function node(key: string, status: PhoneMissionNode["status"], over: Partial<PhoneMissionNode> = {}): PhoneMissionNode {
  return {
    key, title: `Step ${key}`, kind: "change", depends_on: [], status, blocked: null, delivery: "pr", pr: null, pr_state: null, base_pr: null, stage: null,
    attention: null, landing: null, agent_runtime: "running", worker_agent: "claude", agent: null, escalation: null, ...over,
  };
}

function mission(id: string, over: Partial<PhoneMission> = {}): PhoneMission {
  return {
    id, title: `Mission ${id}`, device_id: "local", device_label: null, status: "running", decision: null, question: null, answer: null, answer_failed: null,
    landing_order: [], created_at_unix_ms: 1_000, started_at_unix_ms: 1_000, closed_at_unix_ms: null, stale: false,
    conductor: { present: true, takes_answers: true, name: "Conductor", agent_kind: "claude", agent: null }, nodes: [], ...over,
  };
}

/** A phone paired and connected to a fake hided that has listed the given missions. */
async function connect(items: PhoneMission[]) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  vi.stubGlobal("WebSocket", FakeSocket);
  FakeSocket.instances = [];
  vi.resetModules();
  const { App } = await import("./App");
  const connection = await import("./connection");
  const store = await import("./store");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  store.patch({ screen: "pair", pair: { v: 1, endpoint: "https://mac.example.ts.net", code: "code" } });
  connection.pairNow();
  const socket = FakeSocket.instances[0]!;
  const serve = (frame: ServerFrame) => act(async () => socket.onmessage?.({ data: JSON.stringify(frame) }));
  await act(async () => socket.onopen?.());
  await serve({ type: "hello", mac_name: "Mac", phone_id: "phone-a", name: null, vapid_public_key: "key", notifications: "off" });
  await serve({ type: "agents", groups: [], devices: [], interface_language: "en", color_theme: null });
  await serve({ type: "missions", items, failed: false });
  await act(async () => root.render(<App />));
  const click = (selector: string) => act(async () => container.querySelector<HTMLElement>(selector)!.click());
  const answers = () => socket.sent.filter((message) => message.type === "answer_mission");
  return { container, socket, serve, click, answers, connection, store, unmount: () => act(async () => root.unmount()).then(() => container.remove()) };
}

// The first import of the app transforms its whole module graph, seconds on a busy machine; that cost is paid in this hook, with the
// hook's own budget, so the first test's budget is for what it asserts.
beforeAll(async () => {
  vi.stubGlobal("WebSocket", FakeSocket);
  await import("./App");
  vi.unstubAllGlobals();
});

afterEach(() => {
  vi.unstubAllGlobals();
});


const HOUR = 60 * 60_000;
const gate = (state: "ahead" | "open" | "passed", look: string, over: Record<string, unknown> = {}) => ({ state, look, ask: null, since_unix_ms: null, long_wait_at_unix_ms: null, via: null, said: null, ...over });

it("counts the gates still ahead on the list row in grey, lists them over a plan that waits for approval, and draws each step's strip as the desktop does", async () => {
  const since = Date.now() - 26 * HOUR;
  const items = [
    mission("m-plan", { status: "planning", started_at_unix_ms: null, nodes: [node("a", "planned", { gate: gate("ahead", "the cause written down") }), node("b", "planned", { gate: gate("ahead", "two screen drafts") })] }),
    mission("m-run", { created_at_unix_ms: 900, nodes: [
      node("a", "done", { gate: gate("passed", "the cause written down", { via: "phone", said: "Go on" }) }),
      node("b", "working", { gate: gate("open", "two screen drafts", { since_unix_ms: since, long_wait_at_unix_ms: since + 24 * HOUR }) }),
      node("c", "planned", { gate: gate("ahead", "the merged screen") }),
    ] }),
  ];
  const { container, click, unmount } = await connect(items);
  try {
    await click('[data-phone-tab="missions"]');
    expect(container.querySelector('[data-phone-mission="m-plan"] [data-mission-row-gates]')?.getAttribute("aria-label")).toBe("Where you look 2");
    expect(container.querySelector('[data-phone-mission="m-run"] [data-mission-row-gates]')?.getAttribute("data-mission-row-gates")).toBe("1");
    // A gate ahead is not a turn: the tab counts none.
    expect(container.querySelector('[data-phone-tab="missions"]')!.getAttribute("data-phone-tab-count")).toBeNull();

    await click('[data-phone-mission="m-plan"]');
    expect([...container.querySelectorAll("[data-mission-gate-row]")].map((row) => row.textContent)).toEqual([
      "Step athe cause written downOnce all other finish lines pass",
      "Step btwo screen draftsOnce all other finish lines pass",
    ]);
    await click('[data-phone-back="missions"]');

    await click('[data-phone-mission="m-run"]');
    expect(container.querySelector("[data-mission-gates]")).toBeNull();
    expect(container.querySelector('[data-mission-node="b"] [data-node-gate]')?.getAttribute("data-node-gate")).toBe("open");
    expect(container.querySelector('[data-mission-node="b"] [data-node-gate-long]')?.textContent).toBe("· Waiting long");
    expect(container.querySelector('[data-mission-node="c"] [data-node-gate]')?.getAttribute("data-node-gate")).toBe("ahead");
    expect(container.querySelector('[data-mission-node="c"] [data-node-gate-long]')).toBeNull();
    expect(container.querySelector('[data-mission-node="a"] [data-node-gate]')?.getAttribute("data-node-gate")).toBe("passed");
    expect(container.querySelector('[data-mission-node="a"] [data-phone-node-gate-via="phone"]')?.textContent).toBe("Answered on the phone · “Go on”");
  } finally {
    await unmount();
  }
});

it("keeps the later planned review visible after a two-step plan starts, without adding a turn", async () => {
  const planned = mission("m-start", { status: "planning", started_at_unix_ms: null, nodes: [
    node("first", "planned"),
    node("later", "planned", { depends_on: ["first"], gate: gate("ahead", "the finished screen") }),
  ] });
  const { container, click, serve, unmount } = await connect([planned]);
  try {
    await click('[data-phone-tab="missions"]');
    await click('[data-phone-mission="m-start"]');
    expect(container.querySelector("[data-mission-gate-row]")?.textContent).toContain("the finished screen");
    await serve({ type: "missions", failed: false, items: [{ ...planned, status: "running", started_at_unix_ms: 2_000, nodes: [
      { ...planned.nodes[0]!, status: "working" },
      planned.nodes[1]!,
    ] }] });
    const later = container.querySelector('[data-mission-node="later"]');
    expect(later?.textContent).toContain("Step later");
    expect(later?.querySelector('[data-node-gate="ahead"]')?.textContent).toBe("the finished screen");
    // It is not begun: it says what it waits for, not where its work stands.
    expect(later?.getAttribute("data-node-status")).toBe("planned");
    expect(container.querySelector("[data-dependency-graph]")?.getAttribute("data-dependency-edges")).toBe("2");
    await click('[data-phone-back="missions"]');
    expect(container.querySelector('[data-phone-tab="missions"]')!.getAttribute("data-phone-tab-count")).toBeNull();
    expect(container.querySelector('[data-phone-mission="m-start"] [data-mission-row-gates]')?.textContent).toBe("1");
  } finally {
    await unmount();
  }
});
