// @vitest-environment jsdom
// The phone's missions tab and mission page (docs/UI_BEHAVIOR.md, The phone
// app; PRD mobile-companion-ui B28, B31, B33-B36, B45): the desktop's four
// groups, the answer to a Conductor decision as one request, and the reasons
// a refused answer shows in the desktop's words.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
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

/** A mission waiting on a decision with two choices, the second step stopped by it. */
function deciding(over: Partial<PhoneMission> = {}): PhoneMission {
  return mission("m-turn", {
    decision: "Merge now?",
    question: { key: 4, why: "CI is green", options: ["Merge", "Wait"], recommend: 1, node: "b", asked_at_unix_ms: 5_000 },
    nodes: [node("a", "done", { pr: 1, pr_state: "merged" }), node("b", "working")],
    ...over,
  });
}

const closed = (id: string, at: number, status: PhoneMission["status"] = "done") => mission(id, { status, created_at_unix_ms: at - 10, closed_at_unix_ms: at, nodes: [node("a", "done")] });

/** A phone paired and connected to a fake hided that has listed the given missions. */
async function connect(items: PhoneMission[]) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
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

afterEach(() => {
  vi.unstubAllGlobals();
});

const typeInto = (input: HTMLInputElement, value: string) =>
  act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });

it("lists missions in the desktop's four groups, badges the tab with the missions waiting on the operator, and folds 끝남 to the last three", async () => {
  const items = [
    deciding(),
    mission("m-run", { nodes: [node("a", "done"), node("b", "working"), node("c", "planned"), node("d", "planned")] }),
    mission("m-merge", { status: "merge_wait", nodes: [node("a", "done", { pr: 7, pr_state: "mergeable" })] }),
    closed("c1", 9_000),
    closed("c2", 8_000),
    closed("c3", 7_000, "cancelled"),
    closed("c4", 6_000),
  ];
  const { container, click, unmount } = await connect(items);
  try {
    const tab = container.querySelector('[data-phone-tab="missions"]')!;
    expect(tab.getAttribute("data-phone-tab-count")).toBe("1");
    await click('[data-phone-tab="missions"]');

    const groups = [...container.querySelectorAll("[data-phone-mission-group]")];
    expect(groups.map((group) => [group.getAttribute("data-phone-mission-group"), group.getAttribute("data-phone-mission-group-count")])).toEqual([
      ["turn", "1"], ["running", "1"], ["merge_wait", "1"], ["closed", "4"],
    ]);
    // The row says what waits, and counts the steps done of all.
    expect(container.querySelector('[data-phone-mission="m-turn"] [data-phone-mission-line]')!.getAttribute("data-phone-mission-line")).toBe("decision");
    expect(container.querySelector('[data-phone-mission="m-turn"] [data-phone-mission-counts]')!.textContent).toBe("1/2");
    expect(container.querySelector('[data-phone-mission="m-run"] [data-phone-mission-counts]')!.textContent).toBe("1/4");

    const shown = () => [...container.querySelectorAll('[data-phone-mission-state="closed"]')].map((row) => row.getAttribute("data-phone-mission"));
    expect(shown()).toEqual(["c1", "c2", "c3"]);
    await click('[data-phone-fold="closed"]');
    expect(shown()).toEqual(["c1", "c2", "c3", "c4"]);
  } finally {
    await unmount();
  }
});

it("says there are no missions when the record is empty, and draws nothing for a record the core could not read but the unreadable line", async () => {
  const { container, click, serve, unmount } = await connect([]);
  try {
    await click('[data-phone-tab="missions"]');
    expect(container.querySelector('[data-phone-missions-state="empty"]')!.textContent).toContain("/hide-mission");
    await serve({ type: "missions", items: [], failed: true });
    expect(container.querySelector('[data-phone-missions-state="unreadable"]')).not.toBeNull();
    expect(container.querySelector("[data-phone-mission-group]")).toBeNull();
  } finally {
    await unmount();
  }
});

it("answers a decision with one request, and the same press again reuses that request id", async () => {
  const { container, click, answers, connection, unmount } = await connect([deciding()]);
  try {
    await click('[data-phone-tab="missions"]');
    await click('[data-phone-mission="m-turn"]');
    const page = container.querySelector('[data-phone-mission-page="m-turn"]')!;
    // The question, its reason and the recommended choice come first; the stopped step is held.
    expect(page.querySelector("[data-phone-question]")!.textContent).toBe("Merge now?");
    expect(page.querySelector('[data-phone-option="1"]')!.hasAttribute("data-phone-recommended")).toBe(true);
    expect(page.querySelector('[data-phone-step-kind="hold"]')!.getAttribute("data-phone-step")).toBe("b");
    expect(container.querySelector("[data-phone-tabs]")).toBeNull();

    await click('[data-phone-option="1"]');
    expect(answers()).toHaveLength(1);
    expect(answers()[0]).toMatchObject({ type: "answer_mission", mission: "m-turn", key: 4, option: 1 });
    expect(container.querySelector("[data-phone-decision]")!.getAttribute("data-phone-decision")).toBe("sending");
    expect(container.querySelector("[data-phone-option]")).toBeNull();

    // The page draws the core's state again after a while; the press that was never confirmed goes again under the same id.
    await act(async () => connection.settleMissionAnswer());
    await click('[data-phone-option="1"]');
    expect(answers()).toHaveLength(2);
    expect(answers()[1]!.request_id).toBe(answers()[0]!.request_id);
  } finally {
    await unmount();
  }
});

it("keeps the written answer when it is refused, shows why in words, and sends it again as a new request", async () => {
  const { container, click, serve, answers, unmount } = await connect([deciding()]);
  try {
    await click('[data-phone-tab="missions"]');
    await click('[data-phone-mission="m-turn"]');
    const input = container.querySelector<HTMLInputElement>("[data-phone-answer-input]")!;
    await typeInto(input, "  merge after lunch ");
    await click("[data-phone-answer-send]");
    expect(answers()[0]).toMatchObject({ mission: "m-turn", key: 4, text: "merge after lunch" });
    expect(answers()[0]).not.toHaveProperty("option");

    await serve({ type: "mission_result", request_id: answers()[0]!.request_id as string, mission: "m-turn", ok: false, reason: "gone" });
    expect(container.querySelector("[data-phone-answer-error]")!.textContent).toContain("no longer open");
    expect(container.querySelector<HTMLInputElement>("[data-phone-answer-input]")!.value).toBe("  merge after lunch ");

    // Nothing reached the Conductor, so pressing again is a new request, not a repeat.
    await click("[data-phone-answer-send]");
    expect(answers()).toHaveLength(2);
    expect(answers()[1]!.request_id).not.toBe(answers()[0]!.request_id);
    await serve({ type: "mission_result", request_id: answers()[1]!.request_id as string, mission: "m-turn", ok: true, reason: null });
    expect(container.querySelector<HTMLInputElement>("[data-phone-answer-input]")!.value).toBe("");
  } finally {
    await unmount();
  }
});

it("shows the desktop's reason and presses nothing when the Conductor is gone or the Mac is out of reach", async () => {
  const { container, click, answers, unmount } = await connect([deciding({ conductor: { present: false, takes_answers: true, name: "Conductor", agent_kind: "claude", agent: null } })]);
  try {
    await click('[data-phone-tab="missions"]');
    await click('[data-phone-mission="m-turn"]');
    expect(container.querySelector('[data-phone-answer-reason="no_conductor"]')!.textContent).toBe("No Conductor");
    const options = [...container.querySelectorAll<HTMLButtonElement>("[data-phone-option]")];
    expect(options).toHaveLength(2);
    expect(options.every((option) => option.disabled)).toBe(true);
    expect(container.querySelector<HTMLInputElement>("[data-phone-answer-input]")!.disabled).toBe(true);
    expect(container.querySelector<HTMLButtonElement>("[data-phone-answer-send]")!.disabled).toBe(true);
    await click("[data-phone-option]");
    expect(answers()).toHaveLength(0);
  } finally {
    await unmount();
  }
  const out = await connect([deciding()]);
  try {
    await out.click('[data-phone-tab="missions"]');
    await out.click('[data-phone-mission="m-turn"]');
    await act(async () => out.store.patch({ connected: false, unreachable: true }));
    expect(out.container.querySelector("[data-phone-unreachable]")).not.toBeNull();
    expect([...out.container.querySelectorAll<HTMLButtonElement>("[data-phone-option]")].every((option) => option.disabled)).toBe(true);
    expect(out.container.querySelector<HTMLInputElement>("[data-phone-answer-input]")!.disabled).toBe(true);
  } finally {
    await out.unmount();
  }
});
