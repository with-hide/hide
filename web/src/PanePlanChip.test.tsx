// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { initializeInterfaceI18n } from "./i18n/instance";
import { PanePlanChip } from "./PanePlanChip";
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

it("shows n/m while the plan runs, 검토 while it waits for review, green m/m once finished, and nothing without a plan (B7, B21)", async () => {
  const running = await mount([plan()]);
  try {
    expect(running.chip()?.textContent).toBe("1/3");
    expect(running.chip()?.getAttribute("data-plan-chip-state")).toBe("running");
  } finally { await running.unmount(); }
  const proposed = await mount([plan({ review: "proposed" })]);
  try {
    expect(proposed.chip()?.textContent).toBe("Review");
    expect(proposed.chip()?.getAttribute("aria-label")).toBe("Plan waiting for review · 3 steps");
  } finally { await proposed.unmount(); }
  const finished = await mount([plan({ progress: { current: null, next: null, done: 3, total: 3, finished: true } })]);
  try {
    expect([finished.chip()?.textContent, finished.chip()?.getAttribute("data-plan-chip-state")]).toEqual(["3/3", "finished"]);
  } finally { await finished.unmount(); }
  const none = await mount([]);
  try {
    expect(none.chip()).toBeNull();
  } finally { await none.unmount(); }
});

it("opens a small map, never a list, whose Larger opens the plan in the 보기 slot (B7, B11, D-29)", async () => {
  const view = await mount([plan()]);
  try {
    expect(view.popover()).toBeNull();
    await act(async () => view.chip()!.focus());
    const steps = [...view.popover()!.querySelectorAll<HTMLElement>("[data-plan-step]")];
    expect(steps.map((node) => [node.dataset.planStep, node.dataset.stepStatus])).toEqual([["spec", "done"], ["core", "working"], ["docs", "dropped"], ["web", "planned"]]);
    expect(view.popover()!.querySelector("[data-plan-graph=compact]")).not.toBeNull();
    expect(view.popover()!.querySelector("[data-step-current]")?.getAttribute("data-plan-step")).toBe("core");
    expect(view.popover()!.querySelector("[data-plan-summary]")?.textContent).toBe("Plan1/3· updated 5m ago");
    await act(async () => view.popover()!.querySelector<HTMLButtonElement>("[data-pane-plan-large]")!.click());
    expect(view.events.at(-1)).toEqual({ schema_version: 2, kind: "detail_open", payload: { item: { kind: "plan", session: SESSION } } });
  } finally { await view.unmount(); }
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
