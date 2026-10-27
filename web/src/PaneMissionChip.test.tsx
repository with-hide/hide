// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { initializeInterfaceI18n } from "./i18n/instance";
import { revealFirstPlan } from "./missionReveal";
import { PaneWorkChip } from "./PaneMissionChip";
import type { AgentRow, Mission, MissionNode, PlanView, SnapshotRest } from "./snapshot";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";

// The Conductor pane's mission chip and its popover (PRD mission-pane-chip
// B1-B4, B10, B12-B14, B17; PRD mission-plan-visibility B7-B11, B36): what
// the chip reads, how the popover opens, pins and closes, what it lists while
// the mission runs and once it ends, where its controls lead, and when a
// first plan opens it.

// The layout worker is not under test here: the graph stays in its plain layers.
vi.mock("./dependencyLayout", async (original) => ({
  ...(await original<typeof import("./dependencyLayout")>()),
  workerElk: () => Promise.reject(new Error("no layout worker in the test")),
}));

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
const savedShell = useShellStore.getState();
const savedUi = useUiStore.getState();
afterEach(() => {
  useShellStore.setState(savedShell, true);
  useUiStore.setState(savedUi, true);
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const SESSION = { device_id: "mac", agent: "claude", session_id: "s-1" };

function node(key: string, extra: Partial<MissionNode> = {}): MissionNode {
  return { key, title: `${key} title`, kind: "change", depends_on: [], status: "planned", blocked: null, delivery: "pr", worker: null, agent_id: null, pane_id: null, machine: null, pr: null, base_pr: null, branch: null, pr_state: null, pr_url: null, stage: null, attempts: 0, line: null, change: null, review: null, criteria: [], criteria_met: 0, criteria_skipped: 0, external: null, unread: false, reopened: 0, started_at_unix_ms: null, finished_at_unix_ms: null, commit: null, verdict: null, checks: null, ...extra };
}

/** A pull request number and how the card writes it, built so no literal reads as a colour. */
const PR = 1504;
const PR_TEXT = `#${PR}`;
const NODES = [node("spec", { status: "done" }), node("api", { status: "working", pr: PR, depends_on: ["spec"] }), node("ui", { depends_on: ["api"] })];

function mission(extra: Partial<Mission> = {}): Mission {
  return { id: "mission-1", title: "Ship the API", workspace_id: "w", project: "/repo", checkout_path: "/repo", device_id: "mac", status: "running", scope: "pr", cap: 4, exempt: false, strength: null, other_vendor: null, conductor: { pane_id: "p1", name: "conductor", agent_kind: "claude", session: SESSION, present: true, takes_answers: true }, nodes: NODES, merge_order: [], check: null, github: null, decision: null, question: null, answer: null, answer_failed: null, delay: null, report: null, created_at_unix_ms: 1, started_at_unix_ms: 1, closed_at_unix_ms: null, running: 1, stale: false, github_stale: false, github_read_at_unix_ms: null, ...extra };
}

function agent(pane: string): AgentRow {
  return legacyAgentRow({ pane_id: pane, agent_kind: "claude", session_id: `s-${pane}`, device_id: "mac", identity_label: "root", group: "working", activity: "working", status_code: "working" } as Partial<AgentRow>);
}

function plan(): PlanView {
  return { pane_id: "p1", session: SESSION, steps: [], review: "approved", ask: null, progress: { current: null, next: null, done: 0, total: 1, finished: false }, created_at_unix_ms: 1, updated_at_unix_ms: 1 };
}

function snapshot(missions: Mission[], plans: PlanView[] = []): SnapshotRest {
  return {
    navigator: { devices: [{ id: "mac", kind: "local" }], workspaces: [{ id: "w", checkouts: [{ id: "c", workspace_id: "w", path: "/repo", tabs: [] }] }] },
    missions: { items: missions },
    plans: { items: plans },
  } as unknown as SnapshotRest;
}

async function mount(missions: Mission[], { pane = "p1", plans = [] as PlanView[] } = {}) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  initializeInterfaceI18n("en");
  useShellStore.setState({ agents: [agent("p1"), agent("p2")], focusedPaneId: pane, rest: snapshot(missions, plans) });
  useUiStore.setState({ planPane: null });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><PaneWorkChip paneId={pane} actions={actions} /></TooltipProvider>));
  const chip = () => container.querySelector<HTMLButtonElement>("[data-pane-mission-chip]");
  const popover = () => document.querySelector<HTMLElement>("[data-pane-mission]");
  const press = async (selector: string) => act(async () => popover()!.querySelector<HTMLButtonElement>(selector)!.click());
  const escape = async () => act(async () => { document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); });
  return { container, actions, events, chip, popover, press, escape, unmount: async () => { await act(async () => root.unmount()); } };
}

it("reads each state in words with a cell per node, in the plan chip's place, and a pane that conducts nothing keeps its plan chip (B8, B36, PRD mission-pane-chip B1, B2, B17)", async () => {
  const allDone = NODES.map((row) => ({ ...row, status: "done" as const }));
  const cases: [Partial<Mission>, string, string, string | undefined][] = [
    [{ status: "planning", nodes: [] }, "planning", "Planning", undefined],
    [{}, "running", "Mission1/3", "done working waiting"],
    [{ decision: "Which base for the screen PR?" }, "decision", "1 to decide", "done working waiting"],
    [{ status: "merge_wait" }, "merge_wait", "Waiting to merge", "done working waiting"],
    [{ status: "done", nodes: allDone, closed_at_unix_ms: 9 }, "done", "Mission3/3", "done done done"],
    [{ status: "cancelled", closed_at_unix_ms: 9 }, "cancelled", "Cancelled", "done cancelled cancelled"],
  ];
  for (const [extra, state, text, cells] of cases) {
    const view = await mount([mission(extra)], { plans: [plan()] });
    try {
      expect([view.chip()?.dataset.missionChipState, view.chip()?.textContent]).toEqual([state, text]);
      // One cell per node in the plan, in its state's colour; no cell before the first plan.
      expect(view.chip()?.querySelector<HTMLElement>("[data-progress-cells]")?.dataset.progressCells).toBe(cells);
      // A decision and a done mission keep their colour while the popover is open.
      if (state === "decision") expect(view.chip()?.className).toContain("data-[state=open]:text-mission-hold");
      if (state === "done") expect(view.chip()?.className).toContain("data-[state=open]:text-mission-done");
      expect(view.chip()?.getAttribute("aria-label")).toMatch(/^Mission Ship the API · /);
      // The name says the exact count and the node at work, since the cells are not read out.
      if (state === "running") expect(view.chip()?.getAttribute("aria-label")).toBe("Mission Ship the API · Mission 1/3 now api title");
      expect(view.container.querySelector("[data-pane-plan-chip]")).toBeNull();
    } finally { await view.unmount(); }
  }
  const other = await mount([mission()], { pane: "p2" });
  try {
    expect(other.chip()).toBeNull();
    expect(other.container.textContent).toBe("");
  } finally { await other.unmount(); }
  const unreadable = await mount([mission()]);
  try {
    await act(async () => useShellStore.setState({ rest: { ...snapshot([mission()]), missions: { items: [mission()], failed: "unreadable" } } as unknown as SnapshotRest }));
    expect(unreadable.chip()).toBeNull();
  } finally { await unreadable.unmount(); }
});

it("opens on focus or pointing without taking the keyboard, pins on a press, and a pointer leaving closes only an unpinned map (B3)", async () => {
  const view = await mount([mission()]);
  try {
    expect(view.popover()).toBeNull();
    await act(async () => view.chip()!.focus());
    expect(view.popover()).not.toBeNull();
    expect(document.activeElement).toBe(view.chip());
    await act(async () => view.chip()!.blur());
    expect(view.popover()).toBeNull();
    vi.useFakeTimers();
    // jsdom has no PointerEvent; React derives enter and leave from over and out.
    const point = (type: "pointerover" | "pointerout", relatedTarget: EventTarget | null) => act(async () => { view.chip()!.dispatchEvent(new MouseEvent(type, { bubbles: true, relatedTarget })); });
    await point("pointerover", null);
    expect(view.popover()).not.toBeNull();
    await point("pointerout", document.body);
    await act(async () => { vi.advanceTimersByTime(149); });
    expect(view.popover()).not.toBeNull();
    await act(async () => { vi.advanceTimersByTime(1); });
    expect(view.popover()).toBeNull();
    await act(async () => view.chip()!.click());
    expect(useUiStore.getState().planPane).toBe("p1");
    await point("pointerout", document.body);
    await act(async () => { vi.advanceTimersByTime(500); });
    expect(view.popover()).not.toBeNull();
    await act(async () => view.chip()!.click());
    expect(view.popover()).toBeNull();
  } finally { await view.unmount(); }
});

it("pins on the Show plan chord's action, and the chord again or Esc closes it (B4, D-07)", async () => {
  const view = await mount([mission()]);
  try {
    await act(async () => view.actions.togglePlanChip());
    expect(view.popover()).not.toBeNull();
    await act(async () => view.actions.togglePlanChip());
    expect(view.popover()).toBeNull();
    await act(async () => view.actions.togglePlanChip());
    await view.escape();
    expect([useUiStore.getState().planPane, view.popover()]).toEqual([null, null]);
  } finally { await view.unmount(); }
});

it("lists the nodes a worker is at with their step, one tally line and no map or record footer while the mission runs (B7, B10)", async () => {
  const nodes = [
    node("spec", { status: "done" }),
    node("api", { status: "working", pr: PR, stage: 2, depends_on: ["spec"] }),
    node("probe", { kind: "research", delivery: null, status: "review", depends_on: ["spec"] }),
    node("ui", { depends_on: ["api"] }),
    node("held", { status: "held" }),
  ];
  const view = await mount([mission({ nodes })]);
  try {
    await act(async () => view.actions.togglePlanChip());
    const popover = view.popover()!;
    expect(popover.querySelector("[data-mission-mini-head]")?.textContent).toBe("Mission1/5");
    const rows = [...popover.querySelectorAll<HTMLElement>("[data-mission-mini-node]")].map((row) => [row.dataset.missionMiniNode, row.textContent]);
    expect(rows).toEqual([["api", `api title${PR_TEXT}PR`], ["probe", "probe titleResearch"]]);
    expect(popover.querySelector("[data-mission-mini-tally]")?.textContent).toBe("1 done · 2 next");
    // The map is drawn only in the 보기 slot, and the record opens from the pane menu and the sidebar.
    expect(popover.querySelector("[data-dependency-node]")).toBeNull();
    expect(popover.textContent).not.toContain("record");
    await view.press("[data-mission-mini-large]");
    expect(view.events.at(-1)).toEqual({ schema_version: 2, kind: "detail_open", payload: { item: { kind: "mission", mission: "mission-1" } } });
    expect(view.popover()).toBeNull();
  } finally { await view.unmount(); }
});

it("shows a finished mission's time, the report's first two lines and at most three results by kind, and a cancelled one in grey (B11, B22)", async () => {
  const hour = 3_600_000;
  const nodes = [
    node("one", { status: "done", pr: PR, pr_state: "merged" }),
    node("two", { status: "done", delivery: "direct", branch: "fix/tab" }),
    node("three", { status: "done", kind: "research", delivery: null, line: "the cell resets first" }),
    node("four", { status: "done", kind: "verify", delivery: null, line: "6 of 6 pass" }),
    node("five", { status: "done", kind: "research", delivery: null }),
  ];
  const done = await mount([mission({ status: "done", nodes, started_at_unix_ms: 0, closed_at_unix_ms: 3 * hour, report: "All merged.\nOne step left for later.\nThird line." })]);
  try {
    await act(async () => done.actions.togglePlanChip());
    const popover = done.popover()!;
    expect(popover.querySelector("[data-mission-mini-head]")?.textContent).toBe("Finished· 3h");
    expect(popover.querySelector("[data-mission-mini-report]")?.textContent).toBe("All merged.One step left for later.");
    const results = [...popover.querySelectorAll<HTMLElement>("[data-mission-mini-result]")].map((row) => row.dataset.missionMiniResult);
    expect(results).toEqual(["one", "two", "three"]);
    expect(popover.querySelector("[data-mission-mini-result=one] [data-pr-mark=merged]")).not.toBeNull();
    expect(popover.querySelector("[data-mission-mini-result=two] [data-node-branch]")?.textContent).toBe("fix/tab");
    // A research node with no recorded conclusion left nothing to list.
    expect(popover.querySelector("[data-mission-mini-more]")?.textContent).toBe("+1 more");
    expect(popover.querySelector("[data-mission-mini-tally]")).toBeNull();
  } finally { await done.unmount(); }
  const cancelled = await mount([mission({ status: "cancelled", started_at_unix_ms: 0, closed_at_unix_ms: hour })]);
  try {
    await act(async () => cancelled.actions.togglePlanChip());
    const head = cancelled.popover()!.querySelector<HTMLElement>("[data-mission-mini-head]")!;
    expect([head.dataset.missionMiniHead, head.textContent]).toEqual(["cancelled", "Cancelled· 1h"]);
    expect(head.querySelector(".text-mission-cancelled")).not.toBeNull();
  } finally { await cancelled.unmount(); }
});

it("leads with the decision and its way to the conversation, which brings the Conductor pane forward (B5, B9)", async () => {
  const view = await mount([mission({ decision: "Which base for the screen PR?" })]);
  try {
    await act(async () => view.actions.togglePlanChip());
    const map = view.popover()!;
    expect(map.querySelector("[data-mission-mini-head]")?.textContent).toBe("Mission1/3");
    expect(map.querySelector("[data-mission-mini-decision]")?.textContent).toBe("! Which base for the screen PR?To conversation");
    await view.press("[data-mission-mini-conversation]");
    expect(view.events.at(-1)).toMatchObject({ kind: "focus_pane", payload: { pane_id: "p1" } });
    expect(view.popover()).toBeNull();
  } finally { await view.unmount(); }
});

it("shows one 계획 중 line before the first plan (B10)", async () => {
  const planning = await mount([mission({ status: "planning", nodes: [] })]);
  try {
    await act(async () => planning.actions.togglePlanChip());
    expect(planning.popover()!.querySelector("[data-mission-mini-planning]")?.textContent).toBe("Planning");
    expect(planning.popover()!.querySelector("[data-mission-mini-tally]")).toBeNull();
  } finally { await planning.unmount(); }
});

it("opens the map pinned once when the first plan lands on the pane the operator looks at, and nothing anywhere else (B12-B14)", async () => {
  const view = await mount([mission({ status: "planning", nodes: [] })]);
  try {
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    const focus = vi.spyOn(document, "hasFocus").mockReturnValue(true);
    useUiStore.setState({ screen: { kind: "workspace" }, overviewOpen: false });
    type Shell = ReturnType<typeof useShellStore.getState>;
    const land = (before: Mission, after: Mission, extra: Partial<Shell> = {}, was: Partial<Shell> = {}) => {
      const previous: Shell = { ...useShellStore.getState(), connection: "live", rest: snapshot([before]), ...was };
      revealFirstPlan({ ...previous, rest: snapshot([after]), ...extra }, previous);
    };
    const empty = mission({ status: "planning", nodes: [] });
    const planned = mission({ status: "planning" });
    // Looking at another pane: only the chip changes.
    land(empty, planned, { focusedPaneId: "p2" });
    expect(useUiStore.getState().planPane).toBeNull();
    // A full snapshot, as after a reconnect, is no plan landing.
    land(empty, planned, { viewGeneration: useShellStore.getState().viewGeneration + 1 });
    expect(useUiStore.getState().planPane).toBeNull();
    // A browser tab that does not hold the keyboard, though it never sent a blur.
    focus.mockReturnValue(false);
    land(empty, planned);
    expect(useUiStore.getState().planPane).toBeNull();
    focus.mockReturnValue(true);
    // The first frame after a reconnect resumes as a delta, applied before the link reads live again.
    land(empty, planned, {}, { connection: "reconnecting" });
    expect(useUiStore.getState().planPane).toBeNull();
    // The Overview over the Workspace.
    useUiStore.setState({ overviewOpen: true });
    land(empty, planned);
    expect(useUiStore.getState().planPane).toBeNull();
    useUiStore.setState({ overviewOpen: false });
    // Looking at the Conductor pane: pinned, the keyboard left where it was.
    land(empty, planned);
    expect(useUiStore.getState().planPane).toBe("p1");
    // Closed, a replan opens nothing again.
    useUiStore.getState().setPlanPane(null);
    land(planned, { ...planned, nodes: [...NODES, node("docs")] });
    expect(useUiStore.getState().planPane).toBeNull();
  } finally { await view.unmount(); }
});
