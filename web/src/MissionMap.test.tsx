// @vitest-environment jsdom
// A mission's map in the 보기 slot (PRD software-factory-conductor B15-B21,
// B43): what it shows in each state of the record, and the one event each
// press sends.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { initializeInterfaceI18n } from "./i18n/instance";
import { MissionMap } from "./MissionMap";
import type { Mission, MissionNode, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
const savedShell = useShellStore.getState();
const savedUi = useUiStore.getState();
afterEach(() => {
  useShellStore.setState(savedShell, true);
  useUiStore.setState(savedUi, true);
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

function node(key: string, extra: Partial<MissionNode> = {}): MissionNode {
  return { key, title: `Node ${key}`, kind: "change", depends_on: [], status: "planned", blocked: null, delivery: "pr", worker: null, agent_id: null, pane_id: null, machine: null, pr: null, base_pr: null, branch: null, pr_state: null, pr_url: null, stage: null, attempts: 0, line: null, change: null, review: null, criteria: null, started_at_unix_ms: null, finished_at_unix_ms: null, ...extra };
}

function mission(extra: Partial<Mission> = {}): Mission {
  return { id: "mission-1", title: "Fix the bug", workspace_id: "w", project: "/repo", checkout_path: "/repo", device_id: "mac", status: "running", scope: "pr", cap: 4, conductor: { pane_id: "p1", name: "conductor", agent_kind: "claude", session: null, present: true }, nodes: [], merge_order: [], check: null, github: null, decision: null, delay: null, report: null, created_at_unix_ms: 1, started_at_unix_ms: Date.now() - 180_000, closed_at_unix_ms: null, running: 2, stale: false, github_stale: false, github_read_at_unix_ms: null, ...extra };
}

async function mount(missions: { items: Mission[]; failed?: string }, remote: unknown[] = [], sessions: unknown[] = []) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  initializeInterfaceI18n("en");
  useShellStore.setState({
    agents: [],
    rest: { missions, navigator: { devices: [{ id: "mac", kind: "local", label: "This Mac" }, { id: "mini", kind: "remote", label: "mini" }], workspaces: [] }, status: { remote }, overview_graph: { sessions } } as unknown as SnapshotRest,
  });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><MissionMap missionId="mission-1" actions={actions} /></TooltipProvider>));
  const q = (selector: string) => container.querySelector<HTMLElement>(selector);
  return { container, events, q, unmount: async () => { await act(async () => root.unmount()); } };
}

it("shows one planning line before a plan, and the plan's numbers in its words", async () => {
  const empty = await mount({ items: [mission({ status: "planning", started_at_unix_ms: null })] });
  try {
    expect(empty.q("[data-mission-planning]")?.textContent).toBe("Planning");
    expect(empty.q("[data-mission-banner]")).toBeNull();
  } finally { await empty.unmount(); }
  const running = await mount({ items: [mission({ nodes: [node("a", { status: "done" }), node("b", { status: "working", depends_on: ["a"] })] })] });
  try {
    expect(running.q('[data-mission-stat="agents"]')?.textContent).toBe("Agents2/4");
    expect(running.q('[data-mission-stat="done"]')?.textContent).toBe("Done1/2");
    expect(running.q('[data-mission-stat="elapsed"]')?.textContent).toBe("Elapsed3m");
    expect(running.container.querySelectorAll("[data-mission-node]")).toHaveLength(2);
    expect(running.q("[data-mission-report]")).not.toBeNull();
  } finally { await running.unmount(); }
});

it("shows the decision line only while there is one, and it goes to the Conductor's conversation", async () => {
  const map = await mount({ items: [mission({ decision: "Pick the screen PR's approach", nodes: [node("a")] })] });
  try {
    expect(map.q('[data-mission-banner="decision"]')?.textContent).toContain("To decide · Pick the screen PR's approach");
    await act(async () => map.q("[data-mission-conversation]")!.click());
    expect(map.events.at(-1)).toMatchObject({ kind: "focus_pane", payload: { pane_id: "p1" } });
  } finally { await map.unmount(); }
});

it("offers Take over when no Conductor holds an open mission, which starts an agent asked to take it", async () => {
  const gone = mission({ conductor: { ...mission().conductor, present: false }, nodes: [node("a")] });
  const map = await mount({ items: [gone] });
  try {
    expect(map.q('[data-mission-banner="no-conductor"]')?.textContent).toContain("No Conductor");
    expect(map.q('[data-mission-stat="conductor"]')?.textContent).toBe("Conductornone");
    await act(async () => map.q("[data-mission-takeover]")!.click());
    const start = map.events.find((event) => event.kind === "agent_start_in_checkout");
    expect(start?.payload).toMatchObject({ checkout_path: "/repo", provider: "claude" });
    expect(String((start?.payload as { prompt: string }).prompt)).toContain("hide mission take mission-1");
  } finally { await map.unmount(); }
  const done = await mount({ items: [{ ...gone, status: "done" }] });
  try {
    expect(done.q("[data-mission-banner]")).toBeNull();
  } finally { await done.unmount(); }
});

it("says the record failed instead of a number, and names a gone mission", async () => {
  const failed = await mount({ items: [], failed: "unreadable" });
  try {
    expect(failed.q('[data-mission-state="unreadable"]')?.textContent).toBe("The mission record could not be read");
    expect(failed.q("[data-mission-summary]")).toBeNull();
  } finally { await failed.unmount(); }
  const gone = await mount({ items: [] });
  try {
    expect(gone.q('[data-mission-state="gone"]')).not.toBeNull();
  } finally { await gone.unmount(); }
});

it("dims a mission whose device does not answer and says whose last values it shows", async () => {
  const map = await mount({ items: [mission({ device_id: "mini", stale: true, nodes: [node("a", { status: "working" })] })] });
  try {
    expect(map.q("[data-mission]")?.dataset.missionStale).toBe("true");
    expect(map.q("[data-mission-stale-note]")?.textContent).toBe("mini is not responding · last values");
    expect(map.q("[data-mission-scroll]")?.className).toContain("opacity-(--opacity-dimmed)");
  } finally { await map.unmount(); }
});

it("draws a PR node with its band, its mark, the base of its stack, its steps and the model its pane runs, and a research node with its line and no steps", async () => {
  const nodes = [
    node("api", { pr: 1503, pr_state: "mergeable", stage: 3, status: "review", pane_id: "p9", worker: { agent: "claude", model: "opus", effort: "high", effort_auto: false } }),
    node("screen", { pr: 1504, base_pr: 1503, pr_state: "failed", stage: 2, status: "failed", depends_on: ["api"], line: "CI failed · attempt 2/2", attempts: 2 }),
    node("cause", { kind: "research", delivery: null, status: "done", line: "The cache was stale", worker: { agent: "opencode", effort_auto: false } }),
    node("together", { kind: "verify", delivery: null, status: "planned", depends_on: ["screen"], worker: { agent: "codex", model: "gpt-6-astra", effort_auto: true } }),
  ];
  const map = await mount({ items: [mission({ nodes })] }, [], [{ device_id: "mac", pane_id: "p9", session_digest: "d", launch_model: "sonnet" }]);
  try {
    const screen = map.q('[data-mission-node="screen"]')!;
    expect(screen.className).toContain("border-l-pr-failed");
    expect(screen.querySelector("[data-pr-mark]")?.getAttribute("data-pr-mark")).toBe("failed");
    expect(screen.querySelector("[data-node-base]")?.textContent).toContain(`${"#"}1503`);
    expect(screen.querySelector("[data-node-steps]")?.getAttribute("data-node-steps")).toBe("2");
    // The worker line is the model the pane runs, never the agent's name; a planned node shows the plan's.
    expect(map.q('[data-mission-node="api"] [data-node-worker]')?.getAttribute("data-node-worker")).toBe("sonnet·high");
    expect(map.q('[data-mission-node="together"] [data-node-worker]')?.getAttribute("data-node-worker")).toBe("gpt-6-astra·auto");
    expect(map.q('[data-mission-node="cause"] [data-node-worker]')).toBeNull();
    const cause = map.q('[data-mission-node="cause"]')!;
    expect(cause.querySelector("[data-pr-mark]")).toBeNull();
    expect(cause.querySelector("[data-node-steps]")).toBeNull();
    expect(map.q('[data-mission-node="together"] [data-node-steps]')).toBeNull();
    expect(cause.querySelector("[data-node-line]")?.textContent).toBe("The cache was stale");
    // A planned node waiting on an earlier one reads its kind: the edge says what it waits for.
    expect(map.q('[data-mission-node="together"] [data-node-line]')?.textContent).toBe("Verify together");
    await act(async () => screen.click());
    const detail = document.querySelector<HTMLElement>('[data-mission-detail="screen"]')!;
    expect(detail.textContent).toContain("Attempts2");
    expect(detail.querySelector<HTMLButtonElement>("[data-node-terminal]")?.disabled).toBe(true);
    expect(detail.querySelector<HTMLButtonElement>("[data-node-pr-card]")?.disabled).toBe(true);
  } finally { await map.unmount(); }
});

it("puts the worker's own plan on the card's last line and folded in the node's detail, else the Conductor's line (PRD hide-plan B15, B16)", async () => {
  const own = {
    pane_id: "p2", session: { device_id: "mac", agent: "codex", session_id: "worker-session" },
    steps: [
      { key: "core", title: "Core events", depends_on: [], status: "working" as const, line: null, output: null, changed: false, change: null, waiting: false },
      { key: "web", title: "Web map", depends_on: ["core"], status: "planned" as const, line: null, output: "pr" as const, changed: false, change: null, waiting: true },
    ],
    review: "approved" as const, ask: null, progress: { current: "core", next: "web", done: 0, total: 2, finished: false },
    created_at_unix_ms: 1, updated_at_unix_ms: Date.now() - 12 * 60_000,
  };
  const map = await mount({ items: [mission({ nodes: [
    node("a", { status: "working", branch: "gen-prd/a", stage: 1, line: "Conductor's line", plan: own }),
    node("b", { status: "working", branch: "gen-prd/b", stage: 1, line: "Conductor's line" }),
  ] })] });
  try {
    expect(map.q('[data-mission-node="a"] [data-node-now]')?.textContent).toBe("Now: Core events (0/2)");
    expect(map.q('[data-mission-node="b"] [data-node-now]')?.textContent).toBe("Conductor's line");
    await act(async () => map.q('[data-mission-node="a"]')!.click());
    const detail = document.querySelector<HTMLElement>('[data-mission-detail="a"]')!;
    expect(detail.querySelector("[data-node-step-names]")?.textContent).toBe("Code → PR → Review → Merge");
    expect(detail.querySelector("[data-node-plan]")?.getAttribute("data-node-plan")).toBe("folded");
    expect(detail.querySelector("[data-node-plan-toggle]")?.textContent).toBe("Own planupdated 12m ago");
    await act(async () => detail.querySelector<HTMLButtonElement>("[data-node-plan-toggle]")!.click());
    expect([...detail.querySelectorAll<HTMLElement>("[data-plan-step]")].map((step) => step.dataset.planStep)).toEqual(["core", "web"]);
  } finally { await map.unmount(); }
});
