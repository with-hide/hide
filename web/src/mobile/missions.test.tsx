// @vitest-environment jsdom
// The phone's missions tab and mission page (docs/UI_BEHAVIOR.md, The phone
// app; PRD mobile-companion-ui B28, B31, B33-B36, B45): the desktop's four
// groups, the answer to a Conductor decision as one request, and the reasons
// a refused answer shows in the desktop's words.
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

it("words each turn as the operator's action with no `!`, draws 지휘 다시 열기 as the pill of a mission no Conductor holds, and still opens a row on a press", async () => {
  const items = [
    deciding(),
    mission("m-gone", { conductor: { present: false, takes_answers: false, name: "Conductor", agent_kind: "claude", agent: null }, nodes: [node("a", "done"), node("b", "planned")] }),
  ];
  const { container, click, unmount } = await connect(items);
  try {
    await click('[data-phone-tab="missions"]');
    const pill = (id: string) => container.querySelector(`[data-phone-mission="${id}"] [data-phone-mission-pill]`)!;
    expect([pill("m-turn").getAttribute("data-phone-mission-pill"), pill("m-turn").textContent]).toEqual(["decision", "Answer"]);
    expect([pill("m-gone").getAttribute("data-phone-mission-pill"), pill("m-gone").textContent]).toEqual(["takeover", "Resume command"]);
    for (const id of ["m-turn", "m-gone"]) expect(container.querySelector(`[data-phone-mission="${id}"]`)!.textContent).not.toContain("!");
    await click('[data-phone-mission="m-gone"]');
    expect(container.querySelector('[data-phone-mission-page="m-gone"]')).not.toBeNull();
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

it("says under the list how many missions and, on a finished mission's page, how many steps the Mac left out of the frame", async () => {
  const stripped = mission("c-old", { status: "done", created_at_unix_ms: 100, closed_at_unix_ms: 200, nodes: [], nodes_omitted: 64 });
  const { container, click, serve, unmount } = await connect([mission("m-run", { nodes: [node("a", "working")] }), stripped]);
  try {
    await click('[data-phone-tab="missions"]');
    expect(container.querySelector("[data-phone-missions-omitted]")).toBeNull();
    await serve({ type: "missions", items: [mission("m-run", { nodes: [node("a", "working")] }), stripped], failed: false, omitted: 3 });
    expect(container.querySelector("[data-phone-missions-omitted]")!.textContent).toBe("3 more missions are not shown here · see them on the desktop");
    await click('[data-phone-mission-state="closed"]');
    expect(container.querySelector("[data-phone-mission-steps-omitted]")!.getAttribute("data-phone-mission-steps-omitted")).toBe("64");
    // No count of steps the page does not hold.
    expect(container.querySelector("[data-phone-mission-conductor]")!.textContent).not.toContain("/");
  } finally {
    await unmount();
  }
});

it("opens a step's worker only when the phone's list holds that row", async () => {
  const listed = { device_id: "local", pane_id: "w1:p2" };
  const cut = { device_id: "local", pane_id: "w1:p9" };
  const { container, click, serve, socket, unmount } = await connect([mission("m-run", { conductor: { present: true, takes_answers: true, name: "Conductor", agent_kind: "claude", agent: cut }, nodes: [node("a", "working", { agent: listed }), node("b", "working", { agent: cut })] })]);
  try {
    const row = {
      kind: "agent" as const, ...listed, root_pane_id: listed.pane_id, group: "working" as const, mark: "going" as const, wait: null, descendants_working: 0, agent_kind: "claude",
      title: "worker", place: null, device_label: null, changed_at_unix_ms: null, line: null, status_code: "working" as const, demand: "none", emphasized: false, holds_notification: false,
    };
    await serve({ type: "agents", groups: [{ group: "working", count: 2, rows: [row] }], omitted: 1, devices: [], interface_language: "en", color_theme: null });
    await click('[data-phone-tab="missions"]');
    await click('[data-phone-mission="m-run"]');
    expect(container.querySelector('[data-phone-step="a"] [data-phone-step-open]')).not.toBeNull();
    expect(container.querySelector('[data-phone-step="b"] [data-phone-step-open]')).toBeNull();
    expect(container.querySelector("[data-phone-mission-chat]"), "the Conductor's row was left out too").toBeNull();
    await click('[data-phone-step="a"] [data-phone-step-open]');
    expect(socket.sent.filter((message) => message.type === "open").map((message) => message.pane_id)).toEqual(["w1:p2"]);
  } finally {
    await unmount();
  }
});

it("draws the raises its Conductor has not settled as one safety net card with a line per held step, and folds the lines the Conductor skipped on its own", async () => {
  const now = Date.now();
  const held = ["a", "b", "c", "d", "e"].map((key) => ({ key, title: `Step ${key}`, reason: "worker_ended" as const, pr: null, since_unix_ms: now - 25 * 60_000 }));
  const nodes = held.map((one) => node(one.key, "working", { attention: { reason: "worker_ended", pr: null, to: "conductor", since_unix_ms: one.since_unix_ms, on_row: false, item: false } }));
  const skip = { node: "a", title: "Step a", line: "check", kind: "check" as const, reason: "no check command in this repo", by: "conductor", at_unix_ms: now - 5 * 60_000 };
  const silent = mission("m-net", { nodes, safety_net: { conductor_present: true, since_unix_ms: now - 25 * 60_000, nodes: held }, skipped_alone: [skip] });
  const gone = mission("m-gone", { nodes, conductor: { present: false, takes_answers: false, name: "Conductor", agent_kind: "claude", agent: null }, safety_net: { conductor_present: false, since_unix_ms: now - 25 * 60_000, nodes: held } });
  const { container, click, unmount } = await connect([silent, gone]);
  try {
    await click('[data-phone-tab="missions"]');
    expect(container.querySelector('[data-phone-mission="m-net"] [data-phone-mission-line]')?.textContent).toBe("5 stuck · the Conductor has not settled them yet");
    await click('[data-phone-mission="m-net"]');
    const net = container.querySelector('[data-phone-safety-net="present"]')!;
    expect(net.querySelector("[data-phone-safety-net-head]")?.textContent).toBe("5 stuck items unchanged for 25m");
    expect([...net.querySelectorAll("[data-phone-held-step]")].map((row) => row.textContent)).toEqual(held.map((one) => `${one.title}Worker ended with work left · for 25m`));
    // Under 지금 each held step waits on its Conductor, in the waiting mark, never the operator's turn.
    expect(container.querySelector('[data-phone-step="a"] [data-phone-step-line="hold"]')?.textContent).toBe("Conductor · Worker ended with work left");
    expect(container.querySelector('[data-phone-step="a"] [data-mark]')?.getAttribute("data-mark")).toBe("waiting");
    expect(container.querySelector('[data-phone-mission-decided="folded"]')?.textContent).toContain("Decided by the Conductor 1");
    await click('[data-phone-fold="decided:m-net"]');
    expect(container.querySelector('[data-phone-skip="a:check"]')?.textContent).toBe("Step acheck skipped · no check command in this repoconductor · 5m");
    await click('[data-phone-back="missions"]');
    await click('[data-phone-mission="m-gone"]');
    expect(container.querySelector('[data-phone-safety-net="absent"] [data-phone-safety-net-head]')?.textContent).toBe("No Conductor conducts this mission · 5 stuck");
  } finally {
    await unmount();
  }
});

it("answers a decision with one request once a marked choice is sent with 답하기, and the same answer again reuses that request id (PRD mission-human-gate B19)", async () => {
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

    // A press only marks the row; 답하기 sends it, and cannot before one is marked.
    expect(container.querySelector<HTMLButtonElement>("[data-phone-choice-send]")!.disabled).toBe(true);
    await click('[data-phone-option="1"]');
    expect(answers()).toHaveLength(0);
    await click("[data-phone-choice-send]");
    expect(answers()).toHaveLength(1);
    expect(answers()[0]).toMatchObject({ type: "answer_mission", mission: "m-turn", key: 4, option: 1 });
    expect(container.querySelector("[data-phone-decision]")!.getAttribute("data-phone-decision")).toBe("sending");
    expect(container.querySelector("[data-phone-option]")).toBeNull();

    // The page draws the core's state again after a while; the press that was never confirmed goes again under the same id.
    await act(async () => connection.settleMissionAnswer());
    await click('[data-phone-option="1"]');
    await click("[data-phone-choice-send]");
    expect(answers()).toHaveLength(2);
    expect(answers()[1]!.request_id).toBe(answers()[0]!.request_id);
  } finally {
    await unmount();
  }
});

it("sends another mission's answer while the first one's accepted answer is still waiting for the core's frame", async () => {
  const { container, click, serve, answers, unmount } = await connect([deciding(), deciding({ id: "m-other", title: "Mission other" })]);
  try {
    await click('[data-phone-tab="missions"]');
    await click('[data-phone-mission="m-turn"]');
    await click('[data-phone-option="1"]');
    await click("[data-phone-choice-send]");
    expect(answers()).toHaveLength(1);
    await serve({ type: "mission_result", request_id: answers()[0]!.request_id as string, mission: "m-turn", ok: true, reason: null });

    // Leave the page before the core's frame lands, then answer the other mission.
    await click('[data-phone-back="missions"]');
    await click('[data-phone-mission="m-other"]');
    await click('[data-phone-option="2"]');
    await click("[data-phone-choice-send]");
    expect(answers()).toHaveLength(2);
    expect(answers()[1]).toMatchObject({ mission: "m-other", key: 4, option: 2 });
    expect(container.querySelector('[data-phone-mission-page="m-other"] [data-phone-decision]')!.getAttribute("data-phone-decision")).toBe("sending");
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

it("lists the decision's Views, opening one from its agent's 볼 것, and with none asks for material once and says so from the core's frame", async () => {
  const look = deciding({
    decision_views: [
      { device_id: "local", display_id: "d1", view: "v1", kind: "browser", label: "compare.html", unseen: true, by: "conductor", pane_id: "p-conductor" },
      { device_id: "local", display_id: "d2", view: "v2", kind: "file", label: "notes.md", by: "worker" },
      { device_id: "local", display_id: "d3", view: "v3", kind: "file", label: "old.md", closed: true, by: "conductor" },
    ],
  });
  const { container, click, serve, socket, unmount } = await connect([look]);
  try {
    await click('[data-phone-tab="missions"]');
    await click('[data-phone-mission="m-turn"]');
    const rows = [...container.querySelectorAll<HTMLElement>("[data-phone-decision-view]")];
    expect(rows.map((row) => row.textContent)).toEqual([expect.stringContaining("compare.html"), expect.stringContaining("notes.md"), expect.stringContaining("old.md")]);
    // A row whose opener's agent is closed presses nothing; with Views there is nothing to ask for.
    expect(rows[1]!.tagName).not.toBe("BUTTON");
    // A View closed since the decision linked it stays, marked closed; with its agent closed too it presses nothing (PRD mission-human-gate B20).
    expect(rows[2]!.tagName).not.toBe("BUTTON");
    expect(rows[2]!.textContent).toContain("Closed");
    expect(container.querySelector("[data-phone-material]")).toBeNull();

    await serve({ type: "missions", items: [deciding()], failed: false });
    const asks = () => socket.sent.filter((message) => message.type === "ask_mission_material");
    await click('[data-phone-material="ask"]');
    expect(asks()).toEqual([expect.objectContaining({ mission: "m-turn", key: 4 })]);
    // A refusal is told in the material's own words and the press stays.
    await serve({ type: "mission_result", request_id: asks()[0]!.request_id as string, mission: "m-turn", ok: false, reason: "gone" });
    expect(container.querySelector("[data-phone-answer-error]")!.textContent).toBe("This question is no longer open.");
    expect(container.querySelector('[data-phone-material="ask"]')).not.toBeNull();

    await serve({ type: "missions", items: [deciding({ question: { ...deciding().question!, material_asked: true } })], failed: false });
    expect(container.querySelector('[data-phone-material="asked"]')).not.toBeNull();
    expect(container.querySelector('[data-phone-material="ask"]')).toBeNull();
  } finally {
    await unmount();
  }
});

it("opens a closed View on the Mac first, then its agent's 볼 것 lists it; a refusal or a question that moved on says so (PRD mission-human-gate B20)", async () => {
  const row = { device_id: "local", display_id: "d3", view: "v3", kind: "file", label: "old.md", by: "conductor" as const, pane_id: "w1:p1" };
  const { container, click, serve, socket, unmount } = await connect([deciding({ decision_views: [{ ...row, closed: true }] })]);
  const reopens = () => socket.sent.filter((message) => message.type === "reopen_mission_view");
  try {
    await click('[data-phone-tab="missions"]');
    await click('[data-phone-mission="m-turn"]');
    await click('[data-phone-decision-view="d3"]');
    expect(reopens()).toEqual([expect.objectContaining({ mission: "m-turn", key: 4, view: "v3" })]);
    const requestId = reopens()[0]!.request_id as string;
    await serve({ type: "mission_result", request_id: requestId, mission: "m-turn", ok: true, reason: null });
    // Accepted, the press waits for the row, and a second press sends nothing.
    expect(container.querySelector("[data-phone-decision-view-opening]")).not.toBeNull();
    await click('[data-phone-decision-view="d3"]');
    expect(reopens()).toHaveLength(1);
    // The row open again opens the agent's 볼 것, where the View is listed.
    await serve({ type: "missions", items: [deciding({ decision_views: [{ ...row, display_id: "d11", reopens: [{ request_id: requestId }] }] })], failed: false });
    await serve({ type: "agents", groups: [{ group: "needs_you", count: 1, rows: [{ kind: "agent", device_id: "local", pane_id: "w1:p1", root_pane_id: "w1:p1", group: "needs_you", mark: "look", wait: null, descendants_working: 0, agent_kind: "claude", title: "Conductor", place: null, device_label: null, changed_at_unix_ms: null, line: null, status_code: "idle", demand: "none", emphasized: true, holds_notification: false }] }], devices: [], interface_language: "en", color_theme: null });
    expect(socket.sent.filter((message) => message.type === "open").at(-1)).toMatchObject({ device_id: "local", pane_id: "w1:p1" });
    await serve({ type: "looks", device_id: "local", pane_id: "w1:p1", views: [{ id: "d11", kind: "doc", name: "old.md", unseen: false, at: 1 }], omitted: 0 });
    expect(container.querySelector('[data-phone-view="looks"]')!.getAttribute("data-state")).toBe("active");
    expect(container.querySelector('[data-phone-look="d11"]')).not.toBeNull();
  } finally {
    await unmount();
  }

  const refused = await connect([deciding({ decision_views: [{ ...row, closed: true }] })]);
  const asked = () => refused.socket.sent.filter((message) => message.type === "reopen_mission_view");
  try {
    await refused.click('[data-phone-tab="missions"]');
    await refused.click('[data-phone-mission="m-turn"]');
    await refused.click('[data-phone-decision-view="d3"]');
    const first = asked()[0]!.request_id as string;
    // Another phone's press of the same row refused first leaves this one waiting, until its own is refused too.
    await refused.serve({ type: "missions", items: [deciding({ decision_views: [{ ...row, closed: true, reopens: [{ request_id: "other-phone", refused: true }, { request_id: first }] }] })], failed: false });
    expect(refused.container.querySelector("[data-phone-decision-view-opening]")).not.toBeNull();
    expect(refused.container.querySelector("[data-phone-answer-error]")).toBeNull();
    await refused.serve({ type: "missions", items: [deciding({ decision_views: [{ ...row, closed: true, reopens: [{ request_id: "other-phone", refused: true }, { request_id: first, refused: true }] }] })], failed: false });
    expect(refused.container.querySelector("[data-phone-answer-error]")!.textContent).toBe("It couldn't be opened again on your Mac.");
    expect(refused.container.querySelector("[data-phone-decision-view-opening]")).toBeNull();
    // Pressed again while the question moves on, the press ends on that.
    await refused.click('[data-phone-decision-view="d3"]');
    expect(asked()).toHaveLength(2);
    await refused.serve({ type: "missions", items: [deciding({ question: { ...deciding().question!, key: 5 } })], failed: false });
    expect(refused.container.querySelector("[data-phone-answer-error]")!.textContent).toBe("This question is no longer open.");
    // A press the core never answers for ends once the phone's wait is over.
    await refused.serve({ type: "missions", items: [deciding({ decision_views: [{ ...row, closed: true }] })], failed: false });
    vi.useFakeTimers();
    try {
      await refused.click('[data-phone-decision-view="d3"]');
      expect(refused.container.querySelector("[data-phone-decision-view-opening]")).not.toBeNull();
      await act(async () => vi.advanceTimersByTime(30_000));
      expect(refused.container.querySelector("[data-phone-decision-view-opening]")).toBeNull();
      expect(refused.container.querySelector("[data-phone-answer-error]")!.textContent).toBe("It couldn't be opened again on your Mac.");
    } finally {
      vi.useRealTimers();
    }
  } finally {
    await refused.unmount();
  }
});

it("lists a decision's first three Views and folds the rest behind +N, as the desktop's card does", async () => {
  const views = [1, 2, 3, 4, 5].map((n) => ({ device_id: "local", display_id: `d${n}`, view: `v${n}`, kind: "file" as const, label: `view-${n}.md`, by: "conductor" as const, pane_id: "p-conductor" }));
  const { container, click, unmount } = await connect([deciding({ decision_views: views })]);
  try {
    await click('[data-phone-tab="missions"]');
    await click('[data-phone-mission="m-turn"]');
    const listed = () => [...container.querySelectorAll("[data-phone-decision-view]")].map((row) => row.getAttribute("data-phone-decision-view"));
    expect(listed()).toEqual(["d1", "d2", "d3"]);
    await click("[data-phone-decision-views-more]");
    expect(listed()).toEqual(["d1", "d2", "d3", "d4", "d5"]);
    expect(container.querySelector("[data-phone-decision-views-more]")).toBeNull();
  } finally {
    await unmount();
  }
});
