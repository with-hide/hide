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
import type { AgentRow, PlanDelegation, PlanStep, PlanView, SnapshotRest } from "./snapshot";
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
const step = (key: string, status: PlanStep["status"], depends_on: string[] = [], extra: Partial<PlanStep> = {}): PlanStep => ({ key, title: `${key} title`, depends_on, status, line: null, output: null, changed: false, change: null, waiting: false, delegate: false, evidence: null, delegation: null, ...extra });

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

async function mount(plans: PlanView[], what: "chip" | "map" = "chip", agents: AgentRow[] = [agent()]) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  initializeInterfaceI18n("en");
  useShellStore.setState({
    agents,
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

it("lists every step of a finished plan in the popover with what each left and who did it, under the time it took (PRD hide-plan-delegate B26, B30)", async () => {
  const steps = [
    step("spec", "done", [], { output: "file", evidence: { kind: "file", path: "docs/delegate-spec.md" } }),
    step("core", "done", ["spec"]),
    step("web", "done", ["core"], { output: "pr", evidence: { kind: "pr", repo: "with-hide/hide", number: 1879 }, delegation: handed("reported", { pane_id: null }) }),
    step("review", "done", ["web"], { delegation: handed("reported", { name: "reviewer", agent: "claude", pane_id: null }) }),
    // Done before evidence was recorded: the kind it left, as before.
    step("old", "done", ["spec"], { output: "pr" }),
    step("docs", "dropped", ["spec"], { output: "pr" }),
  ];
  const view = await mount([plan({ steps, progress: { current: null, next: null, done: 5, total: 5, finished: true }, created_at_unix_ms: 0, updated_at_unix_ms: 2 * 3_600_000 })]);
  try {
    await act(async () => view.actions.togglePlanChip());
    const popover = view.popover()!;
    expect(popover.querySelector("[data-plan-summary]")?.textContent).toBe("Plan finished5/5· took 2h");
    const rows = [...popover.querySelectorAll<HTMLElement>("[data-plan-row]")];
    expect(rows.map((row) => [row.dataset.planRow, row.textContent])).toEqual([
      ["spec", "spec titledelegate-spec.md"],
      ["core", "core title"],
      ["old", "old titlePR"],
      ["docs", "docs titlePR"],
      ["web", "web title#1879web-ui"],
      ["review", "review titleReportreviewer"],
    ]);
    // A child that is no longer on screen has a name and no way to its pane.
    expect(popover.querySelector("[data-step-child-pane]")).toBeNull();
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

const CHILD_SESSION = { device_id: "mac", agent: "codex", session_id: "child-1" };
function handed(state: PlanDelegation["state"], extra: Partial<PlanDelegation> = {}): PlanDelegation {
  return { state, attempt: 1, agent_id: "agent-7", name: "web-ui", agent: "codex", pane_id: "c1", session: CHILD_SESSION, ...extra };
}

/** The child's own plan: two of four done, now on `map`, on no pane of the screen. */
function childPlan(): PlanView {
  return {
    pane_id: "",
    session: CHILD_SESSION,
    steps: [step("parts", "done"), step("store", "done", ["parts"]), step("map", "working", ["store"]), step("e2e", "planned", ["map"])].map((one) => ({ ...one, title: one.key })),
    review: "approved",
    ask: null,
    progress: { current: "map", next: "e2e", done: 2, total: 4, finished: false },
    created_at_unix_ms: 1_000,
    updated_at_unix_ms: 2_000,
  };
}

/** The mockup's moment: the spec done with its file, core mine and current, web handed and at work, review still to hand, the PR mine and planned. */
function delegatingPlan(webState: PlanDelegation["state"] = "working"): PlanView {
  return plan({
    steps: [
      step("spec", "done", [], { output: "file", evidence: { kind: "file", path: "docs/delegate-spec.md" } }),
      step("core", "working", ["spec"], { line: "writing tests" }),
      step("web", "working", ["spec"], { output: "pr", delegation: handed(webState) }),
      step("review", "planned", ["core", "web"], { waiting: true, delegate: true }),
      step("pr", "planned", ["review"], { waiting: true, output: "pr" }),
    ],
    progress: { current: "core", next: "review", done: 1, total: 5, finished: false },
  });
}

/** The parent `p1` with the handed child `c1` and one child `c2` no step is handed to. */
function family(): AgentRow[] {
  const parent = legacyAgentRow({ ...agent(), lineage_child_pane_ids: ["c1", "c2"] } as Partial<AgentRow>);
  const child = (pane: string, label: string) => legacyAgentRow({ pane_id: pane, agent_kind: "codex", device_id: "mac", identity_label: label, group: "working", activity: "working", status_code: "working", delegated: true, lineage_parent_pane_id: "p1" } as Partial<AgentRow>);
  return [parent, child("c1", "web-ui"), child("c2", "lint")];
}

it("shows a handed step with its child's current step and name, the short words in the tail, and counts only the children no step holds (PRD hide-plan-delegate B26, B28, B29)", async () => {
  const view = await mount([delegatingPlan(), childPlan()], "chip", family());
  try {
    await act(async () => view.actions.togglePlanChip());
    const popover = view.popover()!;
    const text = (key: string) => popover.querySelector(`[data-plan-row=${key}]`)?.textContent;
    expect(text("spec")).toBe("spec titledelegate-spec.md");
    expect(text("web")).toBe("web title · mapweb-ui");
    expect(text("review")).toBe("review titleTo hand off");
    expect(text("pr")).toBe("pr titlePR");
    // The chip's cells stay by state: the handed step at work is blue like the main's.
    expect(cells(view)).toBe("done working working waiting waiting");
    expect(popover.querySelector("[data-pane-plan-children]")?.textContent).toBe("Delegated 1");
    // The name goes to the child's pane and closes the popover.
    await act(async () => popover.querySelector<HTMLButtonElement>("[data-step-child-pane=c1]")!.click());
    expect(view.events.at(-1)).toMatchObject({ kind: "focus_pane", payload: { pane_id: "c1", origin: "operator" } });
    expect(view.popover()).toBeNull();
  } finally { await view.unmount(); }
  for (const [state, word] of [["reported", "Report in"], ["blocked", "Blocked"], ["closed", "Closed"]] as const) {
    const moved = await mount([delegatingPlan(state), childPlan()], "chip", family());
    try {
      await act(async () => moved.chip()!.focus());
      expect(moved.popover()?.querySelector("[data-plan-row=web] [data-step-delegation]")?.textContent).toBe(word);
    } finally { await moved.unmount(); }
  }
  // With every child held by a step there is no line.
  const held = await mount([delegatingPlan(), childPlan()], "chip", family().filter((row) => row.pane_id !== "c2"));
  try {
    await act(async () => held.chip()!.focus());
    expect(held.popover()?.querySelector("[data-pane-plan-children]")).toBeNull();
  } finally { await held.unmount(); }
});

it("draws every map card in two lines, who and what now, and opens a handed card on its child's checklist and a way to its pane (PRD hide-plan-delegate B25, B27)", async () => {
  const view = await mount([delegatingPlan(), childPlan()], "map", family());
  try {
    const line = (key: string) => view.container.querySelector(`[data-step-line=${key}]`)?.textContent;
    expect(line("spec")).toBe("Me·delegate-spec.md");
    expect(line("core")).toBe("Me·writing tests");
    expect(line("web")).toBe("web-ui·map 2/4");
    expect(line("review")).toBe("To hand off · waits on core title·web title");
    expect(line("pr")).toBe("Me·Planned");
    expect(view.container.querySelector("[data-plan-step=web] [data-step-output]")?.textContent).toBe("PR");
    await act(async () => view.container.querySelector<HTMLElement>("[data-plan-step=web]")!.click());
    const detail = document.querySelector<HTMLElement>("[data-plan-step-detail=web]")!;
    const fields = [...detail.querySelectorAll("dt")].map((dt) => [dt.textContent, dt.nextElementSibling?.textContent]);
    expect(fields).toEqual([["Agent", "web-ui · Codex"], ["State", "Working"], ["Done when", "web-ui's report + its PR number"], ["Waits on", "spec title (Done)"]]);
    const checklist = detail.querySelector<HTMLElement>("[data-plan-child-plan=web-ui]")!;
    expect(checklist.querySelector("p")?.textContent).toBe("web-ui's plan2/4");
    expect([...checklist.querySelectorAll<HTMLElement>("[data-plan-row]")].map((row) => [row.dataset.planRow, row.dataset.stepStatus])).toEqual([["parts", "done"], ["store", "done"], ["map", "working"], ["e2e", "planned"]]);
    await act(async () => detail.querySelector<HTMLButtonElement>("[data-plan-child-go=c1]")!.click());
    expect(view.events.at(-1)).toMatchObject({ kind: "focus_pane", payload: { pane_id: "c1", origin: "operator" } });
    expect(document.querySelector("[data-plan-step-detail=web]")).toBeNull();
  } finally { await view.unmount(); }
});

it("says a child works without a plan, and draws no count and no way to a pane the child does not have (PRD hide-plan-delegate B27, B32)", async () => {
  const view = await mount([delegatingPlan()], "map", family());
  try {
    // No plan of the child: the card says its row's state.
    expect(view.container.querySelector("[data-step-line=web]")?.textContent).toBe("web-ui·In progress");
    await act(async () => view.container.querySelector<HTMLElement>("[data-plan-step=web]")!.click());
    const detail = document.querySelector<HTMLElement>("[data-plan-step-detail=web]")!;
    expect(detail.querySelector("[data-plan-child-plan]")?.textContent).toBe("Working without a plan");
    expect(detail.querySelector("[data-plan-child-go]")).not.toBeNull();
  } finally { await view.unmount(); }
  // Still starting: a name and nothing it cannot know.
  const starting = plan({ steps: [step("spec", "done"), step("web", "working", ["spec"], { delegation: handed("working", { agent_id: null, pane_id: null, session: null }) })], progress: { current: "web", next: null, done: 1, total: 2, finished: false } });
  const early = await mount([starting], "map", family());
  try {
    expect(early.container.querySelector("[data-step-line=web]")?.textContent).toBe("web-ui");
    await act(async () => early.container.querySelector<HTMLElement>("[data-plan-step=web]")!.click());
    const detail = document.querySelector<HTMLElement>("[data-plan-step-detail=web]")!;
    expect(detail.querySelector("[data-plan-child-plan]")).toBeNull();
    expect(detail.querySelector("[data-plan-child-go]")).toBeNull();
  } finally { await early.unmount(); }
});

it("draws no step and no count for a child whose device does not answer, only its row's state (PRD hide-plan-delegate B32)", async () => {
  const away = { ...CHILD_SESSION, device_id: "phone" };
  const handedAway = delegatingPlan();
  handedAway.steps = handedAway.steps.map((one) => (one.delegation ? { ...one, delegation: { ...one.delegation, session: away } } : one));
  const view = await mount([handedAway, { ...childPlan(), session: away }], "map", family());
  try {
    expect(view.container.querySelector("[data-step-line=web]")?.textContent).toBe("web-ui·In progress");
    await act(async () => view.container.querySelector<HTMLElement>("[data-plan-step=web]")!.click());
    const detail = document.querySelector<HTMLElement>("[data-plan-step-detail=web]")!;
    // Neither its last checklist nor a claim that it works without one.
    expect(detail.querySelector("[data-plan-child-plan]")).toBeNull();
  } finally { await view.unmount(); }
  const chip = await mount([handedAway, { ...childPlan(), session: away }], "chip", family());
  try {
    await act(async () => chip.chip()!.focus());
    expect(chip.popover()?.querySelector("[data-plan-row=web]")?.textContent).toBe("web titleweb-ui");
  } finally { await chip.unmount(); }
});

it("leaves out the finished count while a child's device does not answer (PRD hide-plan-delegate B29)", async () => {
  const finishedChild = (device: string) => family().map((row) => (row.pane_id === "c2" ? { ...row, group: "done", device_id: device } : row));
  const here = await mount([delegatingPlan(), childPlan()], "chip", finishedChild("mac"));
  try {
    await act(async () => here.chip()!.focus());
    expect(here.popover()?.querySelector("[data-pane-plan-children]")?.textContent).toBe("Delegated 1· 1 finished");
  } finally { await here.unmount(); }
  const away = await mount([delegatingPlan(), childPlan()], "chip", finishedChild("phone"));
  try {
    await act(async () => away.chip()!.focus());
    expect(away.popover()?.querySelector("[data-pane-plan-children]")?.textContent).toBe("Delegated 1");
  } finally { await away.unmount(); }
});
