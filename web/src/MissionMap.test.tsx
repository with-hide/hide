// @vitest-environment jsdom
// A mission's detail in the 보기 slot (PRD software-factory-conductor B15-B21,
// B43; PRD mission-plan-visibility B12-B17, B19, B20, B22-B24): what it shows
// in each state of the record, and the one event each press sends.
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
  vi.useRealTimers();
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

it("shows one planning line before a plan, and a head of state, time, a cell per node, n / m done and a legend of the states with nodes in it (B12, B13)", async () => {
  const empty = await mount({ items: [mission({ status: "planning", started_at_unix_ms: null })] });
  try {
    expect(empty.q("[data-mission-planning]")?.textContent).toBe("Planning");
    expect(empty.q("[data-mission-pill]")?.textContent).toBe("Planning");
    expect(empty.q("[data-mission-banner]")).toBeNull();
    expect(empty.q("[data-mission-flow]")).toBeNull();
  } finally { await empty.unmount(); }
  const nodes = [node("a", { status: "done" }), node("b", { status: "working", depends_on: ["a"] }), node("c", { status: "review", depends_on: ["a"] }), node("d", { status: "held" }), node("e", { depends_on: ["b"] }), node("f", { status: "dropped" })];
  const running = await mount({ items: [mission({ nodes })] });
  try {
    expect(running.q("[data-mission-pill]")?.textContent).toBe("Running");
    expect(running.q('[data-mission-stat="time"]')?.textContent).toBe("3m so far");
    expect(running.q('[data-mission-stat="done"]')?.textContent).toBe("1 / 5 done");
    expect(running.q("[data-mission-summary] [data-progress-cells]")?.getAttribute("data-progress-cells")).toBe("done hold working review waiting");
    expect(running.q("[data-mission-legend]")?.getAttribute("data-mission-legend")).toBe("done:1 review:1 working:1 hold:1 waiting:1");
    expect(running.q("[data-mission-legend]")?.textContent).toBe("Done1Review1Working1Decision1Waiting1");
    // No PR count, no cap: the head says the state, the time and the cells (B13).
    expect(running.q("[data-mission-summary]")?.textContent).not.toContain("/4");
    expect(running.container.querySelectorAll("[data-mission-node]")).toHaveLength(6);
    expect(running.q("[data-mission-report]")).not.toBeNull();
    // One page: the head, the map, then the flow table.
    const order = [...running.container.querySelectorAll("[data-mission-summary], [data-mission-fit], [data-mission-flow]")].map((part) => (part as HTMLElement).dataset.missionSummary ? "head" : (part as HTMLElement).dataset.missionFit ? "map" : "flow");
    expect(order).toEqual(["head", "map", "flow"]);
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
    expect(map.q("[data-mission-values]")?.className).toContain("opacity-(--opacity-dimmed)");
  } finally { await map.unmount(); }
});

it("draws a card's state as its whole border, with its PR mark, the base of its stack, its steps, badges and the model its pane runs, and a research node with its conclusion and no steps (B14-B16, B22)", async () => {
  const nodes = [
    node("api", { pr: 1503, pr_state: "mergeable", stage: 3, status: "review", pane_id: "p9", worker: { agent: "claude", model: "opus", effort: "high", effort_auto: false } }),
    node("screen", { pr: 1504, base_pr: 1503, pr_state: "failed", stage: 2, status: "failed", depends_on: ["api"], line: "CI failed · attempt 2/2", attempts: 2 }),
    node("cause", { kind: "research", delivery: null, status: "done", line: "The cache was stale", worker: { agent: "opencode", effort_auto: false } }),
    node("together", { kind: "verify", delivery: null, status: "planned", depends_on: ["screen"], worker: { agent: "codex", model: "gpt-6-astra", effort_auto: true } }),
  ];
  const map = await mount({ items: [mission({ nodes })] }, [], [{ device_id: "mac", pane_id: "p9", session_digest: "d", launch_model: "sonnet", launch_effort: null }]);
  try {
    const screen = map.q('[data-mission-node="screen"]')!;
    // The border is the node's state; the PR's own state is only its mark's colour, and no card has a band.
    expect(screen.dataset.nodeTone).toBe("failed");
    expect(screen.className).toContain("border-mission-failed");
    expect(map.container.querySelector('[data-mission-node][class*="border-l-"]')).toBeNull();
    expect(map.q('[data-mission-node="api"]')?.className).toContain("ring-mission-review/20");
    expect(map.q('[data-mission-node="together"]')?.className).toContain("border-dashed");
    expect(screen.querySelector("[data-node-attempts]")?.textContent).toBe("↻2");
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
    expect(cause.querySelector("[data-node-conclusion=folded]")?.getAttribute("title")).toBe("The cache was stale");
    expect(cause.className).toContain("border-mission-done/60");
    // A planned node waiting on an earlier one reads its kind: the edge says what it waits for.
    expect(map.q('[data-mission-node="together"] [data-node-line]')?.textContent).toBe("Verify together");
    // Pressing a card opens its row in the flow table, not a popover (B20).
    await act(async () => screen.click());
    const detail = map.q('[data-flow-row="screen"] [data-mission-detail="screen"]')!;
    expect(detail.textContent).toContain("Attempts2");
    expect(detail.querySelector<HTMLButtonElement>("[data-node-terminal]")?.disabled).toBe(true);
    expect(detail.querySelector<HTMLButtonElement>("[data-node-pr-card]")?.disabled).toBe(true);
    expect(map.q('[data-flow-row="cause"] [data-node-pr-card]')).toBeNull();
  } finally { await map.unmount(); }
});

it("marks a held card with !, says every badge in the card's name, and draws a research or verify line as a result only once the node ended (B16, B22, B36)", async () => {
  const nodes = [
    node("base", { status: "held" }),
    node("retry", { status: "working", attempts: 3, change: "Split from api" }),
    node("reading", { kind: "research", delivery: null, status: "working", line: "Reading 3 of 5 logs" }),
    node("checking", { kind: "verify", delivery: null, status: "working", line: "4 of 6 so far" }),
    node("cause", { kind: "research", delivery: null, status: "done", line: "The cache was stale" }),
  ];
  const map = await mount({ items: [mission({ nodes, decision: "Which base?" })] });
  try {
    const card = (key: string) => map.q(`[data-mission-node="${key}"]`)!;
    expect(card("base").querySelector("[data-node-held]")?.textContent).toBe("!");
    expect(card("base").getAttribute("aria-label")).toBe("Node base · Held · Held · Which base?");
    expect(card("retry").querySelector("[data-node-held]")).toBeNull();
    expect(card("retry").getAttribute("aria-label")).toBe("Node retry · Working · Attempt 3 · Changed by a replan · Split from api");
    // A running research or verify node's line is where it is, said once, never a result.
    for (const key of ["reading", "checking"]) {
      expect(card(key).querySelector("[data-node-conclusion], [data-node-verify]")).toBeNull();
      expect(card(key).querySelectorAll("[data-node-now]")).toHaveLength(1);
    }
    expect(card("cause").querySelector("[data-node-conclusion=folded]")).not.toBeNull();
    // A done research node's row writes its conclusion out under its own label, not under Now.
    await act(async () => card("cause").click());
    const detail = map.q('[data-mission-detail="cause"]')!;
    expect(detail.querySelector("[data-flow-result=conclusion]")?.textContent).toBe("The cache was stale");
    expect(detail.textContent).toContain("ConclusionThe cache was stale");
    expect(detail.textContent).not.toContain("Now");
    await act(async () => card("reading").click());
    expect(map.q('[data-mission-detail="reading"]')?.textContent).toContain("NowReading 3 of 5 logs");
    expect(map.q('[data-mission-detail="reading"] [data-flow-result]')).toBeNull();
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
    const detail = map.q('[data-mission-detail="a"]')!;
    expect(detail.querySelector("[data-node-step-names]")?.textContent).toBe("Code → PR → Review → Merge");
    expect(detail.querySelector("[data-node-plan]")?.getAttribute("data-node-plan")).toBe("folded");
    expect(detail.querySelector("[data-node-plan-toggle]")?.textContent).toBe("Own plan· updated 12m ago");
    // Opening it lists the steps, never a map (PRD mission-plan-visibility B20, D-07).
    await act(async () => detail.querySelector<HTMLButtonElement>("[data-node-plan-toggle]")!.click());
    expect([...detail.querySelectorAll<HTMLElement>("[data-plan-row]")].map((step) => step.dataset.planRow)).toEqual(["core", "web"]);
    expect(detail.querySelector("[data-plan-graph]")).toBeNull();
  } finally { await map.unmount(); }
});

it("draws every node, done ones too, and outlines the ones at work", async () => {
  const nodes = Array.from({ length: 14 }, (_, index) => node(`n-${index}`, { status: index < 6 ? "done" : index < 9 ? "working" : "planned" }));
  const map = await mount({ items: [mission({ nodes })] });
  try {
    expect(map.container.querySelectorAll("[data-mission-node]")).toHaveLength(14);
    expect([...map.container.querySelectorAll<HTMLElement>("[data-node-moving]")].map((card) => card.dataset.missionNode)).toEqual(["n-6", "n-7", "n-8"]);
  } finally { await map.unmount(); }
});

/** A CSS length as the box style writes it. */
const px = (value: number) => `${value}px`;

it("shrinks a map wider than its slot to the slot's width, and 100% draws it at its own size", async () => {
  const width = vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(1800);
  const height = vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(600);
  const room = vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(900);
  const map = await mount({ items: [mission({ nodes: [node("a", { status: "working" }), node("b")] })] });
  try {
    const fit = map.q("[data-mission-fit]")!;
    expect(fit.dataset.missionScale).toBe("0.50");
    // The box holds the shrunk size, so the slot does not scroll sideways.
    expect((fit.firstElementChild as HTMLElement).style.width).toBe(px(900));
    expect((fit.firstElementChild as HTMLElement).style.height).toBe(px(300));
    await act(async () => map.q('[data-mission-zoom-choice="actual"]')!.click());
    expect(map.q("[data-mission-fit]")?.dataset.missionScale).toBe("1.00");
    expect((map.q("[data-mission-fit]")!.firstElementChild as HTMLElement).style.width).toBe(px(1800));
  } finally {
    await map.unmount();
    width.mockRestore();
    height.mockRestore();
    room.mockRestore();
  }
});

it("lists a row per node in order on the mission's time axis, filters it, and a card pressed on the map opens its row whatever the filter hid (B17, B19, B20)", async () => {
  // An open mission's axis runs to the end of the current minute; the clock is held on a minute so the bars measure exactly.
  vi.useFakeTimers({ toFake: ["Date"], now: 1_800_000_000_000 });
  const start = Date.now() + 60_000 - 100 * 60_000;
  const at = (minutes: number) => start + minutes * 60_000;
  const nodes = [
    node("ui", { status: "working", depends_on: ["api"], started_at_unix_ms: at(50) }),
    node("api", { status: "done", started_at_unix_ms: at(0), finished_at_unix_ms: at(50), pr: 1503, pr_state: "merged" }),
    node("cause", { kind: "research", delivery: null, status: "done", started_at_unix_ms: at(0), finished_at_unix_ms: at(25), line: "The cache was stale" }),
    node("fix", { delivery: "direct", status: "planned", depends_on: ["cause"], branch: "fix/cache" }),
    node("check", { kind: "verify", delivery: null, status: "planned", depends_on: ["ui"] }),
  ];
  const map = await mount({ items: [mission({ nodes, started_at_unix_ms: start })] });
  try {
    const rows = () => [...map.container.querySelectorAll<HTMLElement>("[data-flow-row]")].map((row) => row.dataset.flowRow);
    expect(rows()).toEqual(["api", "cause", "ui", "fix", "check"]);
    // The kind in one word beside the title, a PR change saying none.
    expect(map.q('[data-flow-row="cause"] button')?.textContent).toContain("Research");
    expect(map.q('[data-flow-row="fix"] button')?.textContent).toContain("Direct");
    expect(map.q('[data-flow-row="check"] button')?.textContent).toContain("Verify");
    // One bar from start to finish, or to now while it runs, and none before it starts.
    expect(map.q('[data-flow-row="api"] [data-flow-bar]')?.getAttribute("data-flow-bar")).toBe("0.0 50.0");
    expect(map.q('[data-flow-row="ui"] [data-flow-bar]')?.getAttribute("data-flow-bar")).toBe("50.0 50.0");
    expect(map.q('[data-flow-row="fix"] [data-flow-bar]')).toBeNull();
    expect(map.q("[data-mission-flow-axis]")?.getAttribute("data-mission-flow-axis")?.split("|").at(-1)).toBe("now");
    // A mission recorded before its events opens a row to its times, with no list of events.
    await act(async () => map.q('[data-flow-row="api"] button')!.click());
    expect(map.q('[data-mission-detail="api"] [data-flow-times]')?.textContent).toMatch(/^Started .+Finished .+$/);
    await act(async () => map.q('[data-mission-flow-choice="now"]')!.click());
    expect(rows()).toEqual(["ui", "fix", "check"]);
    await act(async () => map.q('[data-mission-flow-choice="finished"]')!.click());
    expect(rows()).toEqual(["api", "cause"]);
    await act(async () => map.q('[data-mission-node="ui"]')!.click());
    expect(map.q("[data-mission-flow]")?.dataset.missionFlow).toBe("all");
    expect(map.q('[data-flow-row="ui"] button')?.getAttribute("aria-expanded")).toBe("true");
    expect(map.q('[data-mission-node="ui"]')?.getAttribute("aria-pressed")).toBe("true");
  } finally { await map.unmount(); }
});

it("puts a finished mission's report and results first, each result by its kind and no PR number on a mission that had none (B22, B23)", async () => {
  const nodes = [
    node("rules", { kind: "research", delivery: null, status: "done", line: "close 12 of 19" }),
    node("bump", { delivery: "direct", status: "done", branch: "release/1.0.0", depends_on: ["rules"] }),
    node("agents", { kind: "verify", delivery: null, status: "done", line: "7 of 7 pass", depends_on: ["bump"] }),
  ];
  const report = "Released 1.0.0.\nNotes published.\nSite deployed.\nFloor raised.";
  const map = await mount({ items: [mission({ status: "done", nodes, report, started_at_unix_ms: 0, closed_at_unix_ms: 3 * 3_600_000 })] });
  try {
    expect(map.q("[data-mission-pill]")?.textContent).toBe("Finished");
    expect(map.q('[data-mission-stat="time"]')?.textContent).toBe("took 3h");
    const order = [...map.container.querySelectorAll("[data-mission-summary], [data-mission-report-text], [data-mission-results], [data-mission-fit], [data-mission-flow]")].map((part) => Object.keys((part as HTMLElement).dataset)[0]);
    expect(order).toEqual(["missionSummary", "missionReportText", "missionResults", "missionFit", "missionFlow"]);
    // A long report shows its first lines until 펼치기.
    expect(map.q("[data-mission-report-text] p")?.textContent).toBe("Released 1.0.0.\nNotes published.\nSite deployed.");
    await act(async () => map.q("[data-mission-report-toggle]")!.click());
    expect(map.q("[data-mission-report-text] p")?.textContent).toBe(report);
    const results = [...map.container.querySelectorAll<HTMLElement>("[data-mission-result-row]")].map((row) => [...row.querySelectorAll("td")].map((cell) => cell.textContent));
    expect(results).toEqual([["Node rules", "Research", "Conclusion: close 12 of 19"], ["Node bump", "Direct", "release/1.0.0"], ["Node agents", "Verify", "7 of 7 pass"]]);
    expect(map.container.querySelector("[data-pr-mark], [data-node-pr]")).toBeNull();
    expect(map.container.textContent).not.toContain("#");
  } finally { await map.unmount(); }
});

it("draws a cancelled mission in grey, its unfinished nodes dashed grey, with no banner to act on (B24)", async () => {
  const gone = { ...mission().conductor, present: false };
  const nodes = [node("a", { status: "done", pr: 1503, pr_state: "merged" }), node("b", { status: "working", depends_on: ["a"] }), node("c", { depends_on: ["b"] })];
  const map = await mount({ items: [mission({ status: "cancelled", conductor: gone, decision: "Keep going?", nodes, started_at_unix_ms: 0, closed_at_unix_ms: 3_600_000 })] });
  try {
    expect(map.q("[data-mission-pill]")?.textContent).toBe("Cancelled");
    expect(map.q("[data-mission-pill]")?.className).toContain("text-mission-cancelled");
    expect([...map.container.querySelectorAll<HTMLElement>("[data-mission-node]")].map((card) => card.dataset.nodeTone)).toEqual(["done", "cancelled", "cancelled"]);
    expect(map.q('[data-mission-node="b"]')?.className).toContain("border-dashed");
    // Its work stopped: the unfinished node rests, and no line from it is lit.
    expect(map.q('[data-mission-node="b"] [data-mark]')?.getAttribute("data-mark")).toBe("quiet");
    expect(map.container.querySelector(".text-mission-working")).toBeNull();
    expect(map.q("[data-mission-legend]")?.getAttribute("data-mission-legend")).toBe("done:1 cancelled:2");
    expect(map.q("[data-mission-banner]")).toBeNull();
    expect(map.q('[data-mission-result-row="a"] [data-pr-mark=merged]')).not.toBeNull();
  } finally { await map.unmount(); }
});
