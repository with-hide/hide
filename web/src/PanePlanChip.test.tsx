// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { initializeInterfaceI18n } from "./i18n/instance";
import { PanePlanChip } from "./PanePlanChip";
import { PlanChecklist } from "./PlanChecklist";
import { PlanMap } from "./PlanMap";
import { planNowLine, planWorkLine } from "./plans";
import type { AgentRow, PlanStep, PlanView, SnapshotRest } from "./snapshot";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";

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
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const SESSION = { device_id: "mac", agent: "claude", session_id: "s-1" };
const step = (key: string, status: PlanStep["status"], depends_on: string[] = [], extra: Partial<PlanStep> = {}): PlanStep => ({ key, title: `${key} title`, depends_on, status, line: null, output: null, changed: false, change: null, waiting: false, ...extra });

function plan(extra: Partial<PlanView> = {}): PlanView {
  return {
    pane_id: "p1",
    session: SESSION,
    steps: [
      step("spec", "done"),
      step("core", "working", ["spec"], { line: "events land" }),
      step("web", "planned", ["core"], { waiting: true, output: "pr" }),
      step("docs", "dropped", ["spec"]),
    ],
    review: "approved",
    ask: null,
    progress: { current: "core", next: "web", done: 1, total: 3, finished: false },
    created_at_unix_ms: 1_000,
    updated_at_unix_ms: Date.now() - 5 * 60_000,
    ...extra,
  };
}

function agent(): AgentRow {
  return legacyAgentRow({ pane_id: "p1", agent_kind: "claude", session_id: "s-1", device_id: "mac", identity_label: "root", group: "working", activity: "working", status_code: "working" } as Partial<AgentRow>);
}

async function mount(plans: PlanView[], what: "chip" | "map" = "chip") {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  initializeInterfaceI18n("en");
  useShellStore.setState({
    agents: [agent()],
    focusedPaneId: "p1",
    rest: { navigator: { devices: [{ id: "mac", kind: "local" }] }, plans: { items: plans } } as unknown as SnapshotRest,
  });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider>{what === "chip" ? <PanePlanChip paneId="p1" actions={actions} /> : <PlanMap session={SESSION} actions={actions} />}</TooltipProvider>));
  const chip = () => container.querySelector<HTMLButtonElement>("[data-pane-plan-chip=p1]");
  const popover = () => document.querySelector<HTMLElement>("[data-pane-plan=p1]");
  return { container, actions, events, chip, popover, unmount: async () => { await act(async () => root.unmount()); } };
}

const cells = (view: { chip: () => HTMLButtonElement | null }) => view.chip()?.querySelector<HTMLElement>("[data-progress-cells]")?.dataset.progressCells;

it("says 계획 with a cell per step and n/m while the plan runs, 검토 while it waits for review, a green m/m once finished, and nothing without a plan (PRD mission-plan-visibility B1, B36; hide-plan B7, B21)", async () => {
  const running = await mount([plan()]);
  try {
    expect(running.chip()?.textContent).toBe("Plan1/3");
    expect(running.chip()?.getAttribute("data-plan-chip-state")).toBe("running");
    // A dropped step has no cell; the current one is blue, the rest still ahead empty.
    expect(cells(running)).toBe("done working waiting");
    // The cells are not read out, so the name carries the exact count and the step.
    expect(running.chip()?.getAttribute("aria-label")).toBe("Plan 1/3, now core title");
  } finally { await running.unmount(); }
  const proposed = await mount([plan({ review: "proposed" })]);
  try {
    expect(proposed.chip()?.textContent).toBe("Review");
    expect(cells(proposed)).toBe("done hold waiting");
    expect(proposed.chip()?.className).toContain("text-mission-hold");
    expect(proposed.chip()?.getAttribute("aria-label")).toBe("Plan waiting for review · 3 steps");
  } finally { await proposed.unmount(); }
  const finished = await mount([plan({ steps: plan().steps.map((one) => (one.status === "dropped" ? one : { ...one, status: "done" as const })), progress: { current: null, next: null, done: 3, total: 3, finished: true } })]);
  try {
    expect([finished.chip()?.textContent, finished.chip()?.getAttribute("data-plan-chip-state")]).toEqual(["Plan3/3", "finished"]);
    expect(cells(finished)).toBe("done done done");
    expect(finished.chip()?.className).toContain("text-mission-done");
  } finally { await finished.unmount(); }
  // A long plan keeps its width: each run of one state is one stretch, and the count stays exact.
  const many = Array.from({ length: 30 }, (_, index) => step(`s${index}`, index < 12 ? "done" : index === 12 ? "working" : "planned"));
  const long = await mount([plan({ steps: many, progress: { current: "s12", next: "s13", done: 12, total: 30, finished: false } })]);
  try {
    expect(long.chip()?.querySelectorAll("[data-progress-cells] > span")).toHaveLength(3);
    expect(long.chip()?.textContent).toBe("Plan12/30");
  } finally { await long.unmount(); }
  const none = await mount([]);
  try {
    expect(none.chip()).toBeNull();
  } finally { await none.unmount(); }
});

it("opens a checklist in the order the steps wait on, never a map or a record footer, whose Larger opens the plan in the 보기 slot (PRD mission-plan-visibility B3, B4, B7; hide-plan B11, D-29)", async () => {
  const view = await mount([plan({ steps: [...plan().steps.slice(0, 3), step("docs", "dropped", ["spec"], { changed: true, change: "folded into web" })] })]);
  try {
    expect(view.popover()).toBeNull();
    await act(async () => view.chip()!.focus());
    const popover = view.popover()!;
    const rows = [...popover.querySelectorAll<HTMLElement>("[data-plan-row]")];
    expect(rows.map((row) => [row.dataset.planRow, row.dataset.stepStatus, row.dataset.stepParallel ?? null])).toEqual([["spec", "done", null], ["core", "working", null], ["docs", "dropped", "true"], ["web", "planned", null]]);
    expect(popover.querySelector("[data-step-current]")?.getAttribute("data-plan-row")).toBe("core");
    expect(popover.querySelector("[data-step-current]")?.textContent).toBe("core titleNow");
    // Every title whole, a PR tail, a dropped step struck through, the changed step's mark reachable by the keyboard.
    expect(popover.querySelector("[data-plan-row=web]")?.textContent).toBe("web titlePR");
    expect(popover.querySelector("[data-plan-row=docs]")?.className).toContain("line-through");
    expect(popover.querySelector<HTMLElement>("[data-plan-row=docs] [data-step-changed]")?.tabIndex).toBe(0);
    expect(popover.querySelector(".truncate")).toBeNull();
    expect(popover.querySelector("[data-plan-graph]")).toBeNull();
    expect(popover.querySelector("[data-pane-plan-record]")).toBeNull();
    expect(popover.querySelector("[data-plan-summary]")?.textContent).toBe("Plan1/3· updated 5m ago");
    await act(async () => popover.querySelector<HTMLButtonElement>("[data-pane-plan-large]")!.click());
    expect(view.events.at(-1)).toEqual({ schema_version: 2, kind: "detail_open", payload: { item: { kind: "plan", session: SESSION } } });
  } finally { await view.unmount(); }
});

it("folds a finished plan's steps into one line with the time it took and what it left (B6)", async () => {
  const steps = [step("spec", "done", [], { output: "file" }), step("core", "done", ["spec"]), step("web", "done", ["core"], { output: "pr" }), step("docs", "dropped", ["spec"], { output: "pr" })];
  const view = await mount([plan({ steps, progress: { current: null, next: null, done: 3, total: 3, finished: true }, created_at_unix_ms: 0, updated_at_unix_ms: 2 * 3_600_000 })]);
  try {
    await act(async () => view.actions.togglePlanChip());
    const popover = view.popover()!;
    expect(popover.querySelector("[data-plan-summary]")?.textContent).toBe("Plan finished3/3· 2h");
    expect(popover.querySelector("[data-plan-done-steps]")?.textContent).toBe("3 steps done");
    expect(popover.querySelector("[data-plan-row]")).toBeNull();
    // A dropped step's output was never made.
    expect([...popover.querySelectorAll<HTMLElement>("[data-plan-output-row]")].map((row) => row.textContent)).toEqual(["spec titlefile", "web titlePR"]);
  } finally { await view.unmount(); }
});

it("lists every step of a finished plan outside the popover, where the plan is read afterwards (B6)", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  initializeInterfaceI18n("en");
  const steps = [step("spec", "done", [], { output: "file" }), step("core", "done", ["spec"]), step("docs", "dropped", ["spec"])];
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><PlanChecklist plan={plan({ steps, progress: { current: null, next: null, done: 2, total: 2, finished: true } })} /></TooltipProvider>));
  try {
    expect(container.querySelector("[data-plan-done-steps]")).toBeNull();
    expect([...container.querySelectorAll<HTMLElement>("[data-plan-row]")].map((row) => [row.dataset.planRow, row.dataset.stepStatus])).toEqual([["spec", "done"], ["core", "done"], ["docs", "dropped"]]);
  } finally { await act(async () => root.unmount()); }
});

it("pins on ⇧⌘U's action and closes on the second, and a pane with no plan pins nothing", async () => {
  const view = await mount([plan()]);
  try {
    await act(async () => view.actions.togglePlanChip());
    expect(useUiStore.getState().planPane).toBe("p1");
    expect(view.popover()).not.toBeNull();
    await act(async () => view.actions.togglePlanChip());
    expect(useUiStore.getState().planPane).toBeNull();
    expect(view.popover()).toBeNull();
  } finally { await view.unmount(); }
  const none = await mount([]);
  try {
    await act(async () => none.actions.togglePlanChip());
    expect(useUiStore.getState().planPane).toBeNull();
  } finally { await none.unmount(); }
});

it("draws the review band with one button to the conversation, and none once the plan is approved (B13)", async () => {
  const proposed = await mount([plan({ review: "proposed", ask: "scope moved" })], "map");
  try {
    const band = proposed.container.querySelector("[data-plan-banner]");
    expect(band?.getAttribute("data-plan-banner")).toBe("review");
    expect(band?.textContent).toBe("Waiting for review · answer in the conversationGo to conversation");
    expect(proposed.container.querySelectorAll("[data-plan-banner] button")).toHaveLength(1);
    expect(proposed.container.querySelector("[data-plan-graph=large]")).not.toBeNull();
  } finally { await proposed.unmount(); }
  const approved = await mount([plan()], "map");
  try {
    expect(approved.container.querySelector("[data-plan-banner]")).toBeNull();
  } finally { await approved.unmount(); }
  const gone = await mount([], "map");
  try {
    expect(gone.container.querySelector("[data-plan-state=gone]")).not.toBeNull();
  } finally { await gone.unmount(); }
});

it("words the one number and the pane's second line from the core's current step (D-08, B7)", () => {
  initializeInterfaceI18n("ko");
  const t = ((key: string, values: Record<string, string>) => `${key}:${values.step}:${values.progress}`) as never;
  expect(planNowLine(plan(), t)).toBe("plan.now:core title:1/3");
  expect(planWorkLine(plan())).toBe("core title · events land");
  // A plan waiting for review leaves the line to the ask, and a finished one has none.
  expect(planWorkLine(plan({ review: "proposed" }))).toBeNull();
  expect(planNowLine(plan({ progress: { current: null, next: null, done: 3, total: 3, finished: true } }), t)).toBeNull();
});
