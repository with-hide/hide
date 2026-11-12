// @vitest-environment jsdom
// A mission's review gates on the desktop (PRD mission-human-gate B1, B2, B4,
// B9, B10, B18, B21-B23; docs/UI_BEHAVIOR.md, The mission screen): the plan's
// `내가 볼 곳 N`, the card's eye strip in each state, the node panel's one card,
// the list row's grey count, and what is and is not the operator's turn.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { initializeInterfaceI18n } from "./i18n/instance";
import { MissionMap } from "./MissionMap";
import { MissionScreen } from "./MissionScreen";
import type { Mission, MissionGate, MissionNode, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { MISSION_ENTRY, useUiStore } from "./ui";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
const savedShell = useShellStore.getState();
const savedUi = useUiStore.getState();
afterEach(() => {
  useShellStore.setState(savedShell, true);
  useUiStore.setState(savedUi, true);
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const HOUR = 60 * 60_000;

function node(key: string, extra: Partial<MissionNode> = {}): MissionNode {
  return { key, title: `Node ${key}`, kind: "change", depends_on: [], status: "planned", blocked: null, delivery: "pr", worker: null, agent_id: null, pane_id: null, machine: null, pr: null, base_pr: null, branch: null, pr_state: null, pr_url: null, stage: null, attempts: 0, line: null, change: null, review: null, criteria: [], criteria_met: 0, criteria_skipped: 0, steps: [], brief_bytes: null, external: null, unread: false, reopened: 0, started_at_unix_ms: null, finished_at_unix_ms: null, commit: null, landing: null, verdict: null, checks: null, attention: null, hide: null, prs: [], ...extra };
}

function gate(state: MissionGate["state"], look: string, extra: Partial<MissionGate> = {}): MissionGate {
  return { state, look, ask: null, since_unix_ms: null, long_wait_at_unix_ms: null, via: null, said: null, ...extra };
}

function mission(extra: Partial<Mission> = {}): Mission {
  return { id: "mission-1", title: "Fix the bug", workspace_id: "w", project: "/repo", checkout_path: "/repo", device_id: "mac", status: "running", scope: "pr", cap: 4, exempt: false, strength: null, conductor: { pane_id: "p1", name: "conductor", agent_kind: "claude", session: null, present: true, takes_answers: true }, nodes: [], merge_order: [], landing_order: [], stack_waiting: 0, check: null, github: null, decision: null, question: null, answer: null, answer_failed: null, delay: null, report: null, created_at_unix_ms: 1, started_at_unix_ms: Date.now() - 180_000, closed_at_unix_ms: null, running: 1, stale: false, github_stale: false, github_read_at_unix_ms: null, ...extra };
}

function stub() {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  initializeInterfaceI18n("en");
}

const navigator = { devices: [{ id: "mac", kind: "local", label: "This Mac" }], focused_device_id: "mac", workspaces: [{ id: "w", label: "repo", device_id: "mac", checkouts: [] }] };

async function mountMap(one: Mission) {
  stub();
  useShellStore.setState({ connection: "live", rest: { missions: { items: [one] }, navigator, status: { remote: [] }, overview_graph: { sessions: [] } } as unknown as SnapshotRest });
  const actions = createActions(() => true);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><MissionMap missionId="mission-1" actions={actions} /></TooltipProvider>));
  const q = (selector: string) => container.querySelector<HTMLElement>(selector);
  const pick = (key: string) => act(async () => q(`[data-mission-node="${key}"]`)!.click());
  return { container, q, pick, unmount: () => act(async () => root.unmount()) };
}

it("lists where the operator will look over a plan that waits for approval, each with what and when, and none of it is their turn (B1, B2, B9)", async () => {
  const planned = mission({
    status: "planning", started_at_unix_ms: null,
    nodes: [node("cause", { gate: gate("ahead", "the cause written down") }), node("screen", { depends_on: ["cause"], gate: gate("ahead", "two screen drafts") }), node("api"), node("gone", { status: "dropped", gate: gate("ahead", "never looked at") })],
  });
  const map = await mountMap(planned);
  try {
    const list = map.q("[data-mission-gates]")!;
    expect(list.getAttribute("aria-label")).toBe("Where you look 2");
    // When is the same for every place, so the head says it once.
    expect([...list.querySelectorAll("[data-mission-gate-when]")].map((when) => when.textContent)).toEqual(["once each step passes its other finish lines"]);
    expect(list.querySelector("[data-mission-gate-row] [data-mission-gate-when]")).toBeNull();
    // A dropped node's place is left out; each place reads what to look at first, then its node whole.
    expect([...list.querySelectorAll("[data-mission-gate-row]")].map((row) => [row.getAttribute("data-mission-gate-row"), row.textContent])).toEqual([
      ["cause", "the cause written downNode cause"],
      ["screen", "two screen draftsNode screen"],
    ]);
    // Every gated card shows its strip without a hover, and none of them is the operator's turn.
    expect(map.q('[data-mission-node="screen"] [data-node-gate]')?.getAttribute("data-node-gate")).toBe("ahead");
    expect(map.q('[data-mission-node="screen"] [data-node-gate]')?.textContent).toBe("two screen drafts");
    expect(map.q('[data-mission-node="api"] [data-node-gate]')).toBeNull();
    expect(map.container.querySelector("[data-node-turn], [data-node-gate-tag], [data-mission-turn]")).toBeNull();
    // A row's node picks it on the map.
    await act(async () => map.q('[data-mission-gate-node="screen"]')!.click());
    expect(map.q("[data-mission-panel]")?.getAttribute("data-mission-panel")).toBe("screen");
    expect(map.q('[data-panel-gate="ahead"]')?.textContent).toContain("two screen drafts");
  } finally { await map.unmount(); }
  // Once the mission runs, the list is gone and the strips stay.
  const running = await mountMap(mission({ nodes: planned.nodes }));
  try {
    expect(running.q("[data-mission-gates]")).toBeNull();
    expect(running.q('[data-mission-node="cause"] [data-node-gate="ahead"]')).not.toBeNull();
  } finally { await running.unmount(); }
});

it("marks a reached gate as the operator's turn on its card, draws no second card in the panel while the decision asks it, and says 오래 기다림 past 24 hours (B4, B10, B22, D-25)", async () => {
  const since = Date.now() - 25 * HOUR;
  const reached = mission({
    decision: "Check two screen drafts and tell me",
    question: { key: 4, why: null, options: ["Go on", "Something to fix"], recommend: null, node: "screen", asked_at_unix_ms: since },
    nodes: [node("screen", { status: "working", gate: gate("open", "two screen drafts", { ask: 4, since_unix_ms: since, long_wait_at_unix_ms: since + 24 * HOUR }) }), node("next", { depends_on: ["screen"] })],
  });
  const map = await mountMap(reached);
  try {
    const card = map.q('[data-mission-node="screen"]')!;
    expect(card.querySelector("[data-node-turn]")).not.toBeNull();
    expect(card.querySelector("[data-node-gate]")?.getAttribute("data-node-gate")).toBe("open");
    expect(card.querySelector('[data-node-gate-tag="open"]')?.textContent).toBe("Needs you · Waiting long");
    expect(map.q('[data-mission-node="next"] [data-node-turn]')).toBeNull();
    // The decision card says it waited long, beside its waiting time.
    expect(map.q("[data-mission-banner-long]")?.textContent).toBe("· Waiting long");
    await map.pick("screen");
    // The panel's own question says it; the gate card and a second `! You` line are not drawn.
    expect(map.q('[data-panel-question="true"]')?.textContent).toBe("Check two screen drafts and tell me");
    expect(map.q('[data-panel-part="gate"]')).toBeNull();
    expect(map.q('[data-panel-part="attention"]')).toBeNull();
  } finally { await map.unmount(); }
  // Within 24 hours nothing says it waited long.
  const fresh = await mountMap({ ...reached, nodes: reached.nodes.map((one) => (one.gate ? { ...one, gate: { ...one.gate, long_wait_at_unix_ms: Date.now() + HOUR } } : one)) });
  try {
    expect(fresh.q('[data-node-gate-tag="open"]')?.textContent).toBe("Needs you");
    expect(fresh.q("[data-mission-banner-long]")).toBeNull();
  } finally { await fresh.unmount(); }
});

it("with no Conductor, says what to look at in 내 차례 and draws the reached gate as the panel's card with 내 차례 · 오래 기다림 (B23)", async () => {
  const since = Date.now() - 30 * HOUR;
  const map = await mountMap(mission({
    conductor: { pane_id: "p1", name: "conductor", agent_kind: "claude", session: null, present: false, takes_answers: false },
    nodes: [node("screen", { status: "working", attention: { reason: "human_gate", pr: null, to: "operator", since_unix_ms: since, on_row: false, item: true, choices: [] }, gate: gate("open", "two screen drafts", { since_unix_ms: since, long_wait_at_unix_ms: since + 24 * HOUR }) })],
  }));
  try {
    expect(map.q('[data-mission-turn-reason="human_gate"] [data-mission-turn-what]')?.textContent).toBe("two screen drafts");
    await map.pick("screen");
    const card = map.q('[data-panel-gate="open"]')!;
    expect(card.querySelector("[data-panel-gate-when]")?.textContent).toBe("Needs you");
    expect(card.querySelector("[data-panel-gate-long]")?.textContent).toBe("· Waiting long");
    expect(card.textContent).toContain("two screen drafts");
    // The raise says the same as the card, so its part is not drawn again.
    expect(map.q('[data-panel-part="attention"]')).toBeNull();
  } finally { await map.unmount(); }
});

it("draws a passed gate grey with a check, and its panel card says where the answer came from with the operator's words (B6, B21)", async () => {
  const map = await mountMap(mission({ nodes: [node("cause", { status: "done", gate: gate("passed", "the cause written down", { via: "pane", said: "ㄱㄱ" }) })] }));
  try {
    expect(map.q('[data-mission-node="cause"] [data-node-gate]')?.getAttribute("data-node-gate")).toBe("passed");
    expect(map.q('[data-mission-node="cause"]')?.getAttribute("aria-label")).toContain("To look at · the cause written down · Looked at");
    expect(map.q("[data-node-turn]")).toBeNull();
    await map.pick("cause");
    expect(map.q("[data-panel-gate-when]")?.textContent).toBe("Looked at");
    expect(map.q('[data-panel-gate-via="pane"]')?.textContent).toBe("Answered in the window · “ㄱㄱ”");
  } finally { await map.unmount(); }
});

it("ends a mission's list row with a grey eye and the number of gates still ahead, and none when none is (B18)", async () => {
  stub();
  const ahead = mission({ id: "mission-1", nodes: [node("a", { gate: gate("ahead", "x") }), node("b", { gate: gate("ahead", "y") }), node("c", { status: "done", gate: gate("passed", "z") })] });
  const none = mission({ id: "mission-2", created_at_unix_ms: 2, nodes: [node("a", { status: "done", gate: gate("passed", "z") })] });
  useShellStore.setState({ connection: "live", rest: { missions: { items: [ahead, none] }, navigator, status: { remote: [] }, overview_graph: { sessions: [] } } as unknown as SnapshotRest });
  useUiStore.setState({ screen: { kind: "missions" }, missions: { ...MISSION_ENTRY } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><MissionScreen actions={createActions(() => true)} /></TooltipProvider>));
  try {
    const count = container.querySelector('[data-mission-row="mission-1"] [data-mission-row-gates]')!;
    expect(count.getAttribute("data-mission-row-gates")).toBe("2");
    expect(count.getAttribute("aria-label")).toBe("Where you look 2");
    expect(count.className).toContain("text-muted-foreground");
    expect(container.querySelector('[data-mission-row="mission-2"] [data-mission-row-gates]')).toBeNull();
    // Gates ahead are not a turn: the mission stands in 진행 중, not 내 차례.
    expect(container.querySelector('[data-mission-row="mission-1"]')?.getAttribute("data-mission-row-group")).toBe("running");
  } finally { await act(async () => root.unmount()); }
});
