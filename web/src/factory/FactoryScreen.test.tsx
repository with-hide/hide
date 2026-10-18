// @vitest-environment jsdom
// The Factory screen's states a browser run cannot reach without GitHub or a
// running worker (PRD software-factory-ui B10, B19, B20): the outside read
// gone stale, an action the engine refused, and the page actions of states a
// held fixture never enters. The summary is the engine's shape; the screen,
// stores and actions are the real ones, and only dispatch is recorded.

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { english } from "../i18n/catalogs";
import { TooltipProvider } from "../components/ui/tooltip";
import { useShellStore } from "../store";
import type { SnapshotRest } from "../snapshot";
import { FACTORY_ENTRY, useUiStore, type FactoryPlace } from "../ui";
import type { DispatchFn } from "../ws";

// The screen as it will be once the Factory opens (`launch.ts`); until then nothing reaches it.
vi.mock("./launch", () => ({ FACTORY_OPEN: true }));
import { InitFailure, factoryMachines } from "./CreateSheet";
import { FactoryScreen } from "./FactoryScreen";
import { REQUEST_ANSWER_TIMEOUT_MS } from "./request";
import type { CardView, DecisionView, FactorySummary, FactoryView, InboxItem, TaskDetail, TaskState } from "./model";

const NOW = Date.now();

function card(task: string, state: TaskState, patch: Partial<CardView> = {}): CardView {
  return { task, ai_decisions: 0, permission_wait: false, recovering: false, display_id: task, column: null, title: `${task} 제목`, summary: "작업 요약", issue: null, issue_url: null, pr: null, worker_runtime: null, resume_at: null, waiting_group: null, stage: 1, track: 1, state, state_label: state, needs_person: false, waiting_for: null, waiting_code: null, waiting_on: [], env_hold: null, stop: null, priority: 0, since: NOW, unread: false, folded: false, archived: false, failures: 0, external: [], revive_until: null, worker_pane: null, worker_label: null, pause_reason: null, ...patch };
}

function factory(patch: Partial<FactoryView> = {}): FactoryView {
  return {
    id: "f1", project: "/fixture", project_name: "fixture", source: "github", verification: "ci", closed: false,
    flow: { before: 0, stuck: 0, moving: 1, done_today: 0 }, my_turn: 1,
    columns: [{ column: "moving", label: "running", cards: [card("T-1", "running", { column: "moving" })] }],
    cancelled: [], graph: { nodes: ["T-1"], edges: [], unrelated: ["T-1"] }, dependencies: [],
    outside_read_at: NOW - 600_000, stale: false, main_broken: false, auto_merge_available: true, merge_mode: "manual", paused: false, observer_mode: "assist", observer_today: 0, observer_limit: 100, observer_capped: false,
    github_block: null, follow_ups: [], follow_ups_open: 0, activity: [], metrics: { finished: 0, person_items_tenths: null, started: 0, start_median_ms: null, ai_decisions: 0, overridden: 0, override_percent: null }, factory_ai: null, workers: [{ agent: "claude", description: "" }], macos_notifications: false, ...patch,
  };
}

const MERGE: InboxItem = {
  group: "merge", kind: "merge", rank: 0, factory: "f1", task: "T-1", display_id: "#12", title: "T-1 제목", project: "fixture", question: null,
  text: "", stopped: null, holding: "merge", outcomes: [], fallback: null, evidence: [], resolve: null, command: null, impact: null, suggestion: "merge", result: "", default_action: null, choices: ["merge", "request-changes", "cancel"], deadline: null, remaining: null, remaining_hours: null, waiting_since: NOW, waiting_days: 0,
  result_code: "merge", unblocks: [], gates: ["manual_mode"], stop: null, env_hold: null, attempts: [], decision_kind: null, observer_reason: null,
};

function decision(value: Partial<DecisionView> & Pick<DecisionView, "id" | "text" | "by">): DecisionView {
  return { recorded_by: value.by, source: null, kind: null, reason: null, at: NOW, overridable: false, changed: null, ...value };
}

let root: Root | null = null;

async function mount(summary: FactorySummary, place: Partial<FactoryPlace> = {}) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  // jsdom lays nothing out, so it has no scrolling to bring the open item into view.
  Element.prototype.scrollIntoView = () => {};
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  useShellStore.setState({ connection: "live", factory: { summary, actions: [] }, factoryTask: null });
  useUiStore.setState({ screen: { kind: "factory", place: { ...FACTORY_ENTRY, ...place } } });
  const container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await act(async () => root!.render(<TooltipProvider><FactoryScreen actions={actions} /></TooltipProvider>));
  return { container, events };
}

afterEach(async () => {
  await act(async () => root?.unmount());
  root = null;
  document.body.replaceChildren();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  delete (Element.prototype as Partial<Element>).scrollIntoView;
});

it("dims the outside read to its last time after three failed reads, and says how long ago while reads succeed (B31, D-46)", async () => {
  const { container } = await mount({ my_turn: 0, factories: [factory({ stale: true })], inbox: [] });
  const read = container.querySelector("[data-factory-outside-read]")!;
  expect(read.getAttribute("data-factory-outside-read")).toBe("stale");
  expect(read.className).toContain("opacity-(--opacity-dimmed)");
  expect(read.textContent).toBe(english["factory.header.lastRead"].replace("{{time}}", new Date(NOW - 600_000).toLocaleTimeString("en", { hour: "2-digit", minute: "2-digit", hourCycle: "h23" })));
  await act(async () => useShellStore.setState({ factory: { summary: { my_turn: 0, factories: [factory()], inbox: [] }, actions: [] } }));
  expect(container.querySelector("[data-factory-outside-read]")!.getAttribute("data-factory-outside-read")).toBe("fresh");
});

it("keeps a refused answer's item in place with its next action in the screen's language (B10, B24)", async () => {
  const { container, events } = await mount({ my_turn: 1, factories: [factory()], inbox: [MERGE] });
  const send = container.querySelector<HTMLButtonElement>("[data-factory-send]")!;
  await act(async () => send.click());
  const sent = events.at(-1) as unknown as { kind: string; payload: { request_id: string; command: { verb: string; task: string } } };
  expect(sent.kind).toBe("factory_action");
  expect(sent.payload.command).toEqual({ verb: "merge", task: "f1/T-1" });
  await act(async () => useShellStore.setState({ factory: { summary: { my_turn: 1, factories: [factory()], inbox: [MERGE] }, actions: [{ request_id: sent.payload.request_id, answer: { ok: false, reason: "main_dirty", next_action: "Commit or stash the changes in the main checkout, then merge" } }] } }));
  const refused = container.querySelector("[data-factory-refused]")!;
  expect(refused.getAttribute("data-factory-refused")).toBe("main_dirty");
  expect(refused.textContent).toBe(english["factory.refusal.main_dirty"]);
  expect(container.querySelectorAll("[data-factory-item]")).toHaveLength(1);
});

function detail(state: TaskState, allowed: string[]): TaskDetail {
  return {
    card: card("T-1", state), factory: "f1", project: "/fixture", goal: "목표", criteria: [], out_of_scope: [], before: [], after: [], attachments: [], pr: null,
    verification: "1/3", attempts: [], decisions: [], questions: [], discoveries: [], gates: [], gate_codes: [], allowed, stop: null, stop_code: null, merge_sha: null, worker_name: null, worktree: null, branch: null,
    worker: null, diagnosis: null, auto_restarts: 0, resting_since: null, pinned_worker: null, ai_picked_worker: null, ai_pick_reason: null, woke_at: null, diagnosed_at: null, diagnosed_from: null,
    checklist: [], report: null, activity: [], follow_ups: [], issue_text: null,
  };
}

it.each([
  ["running", ["pause", "cancel"], "pause cancel"],
  ["paused", ["resume", "cancel"], "resume cancel"],
  ["merge_waiting", ["merge", "request-changes", "cancel"], "merge request-changes cancel"],
  ["stopped", ["retry", "cancel"], "retry cancel"],
  ["verifying", ["cancel"], "cancel"],
] as const)("draws only the %s state's actions the engine allows (B19)", async (state, allowed, drawn) => {
  const { container, events } = await mount({ my_turn: 0, factories: [factory()], inbox: [] }, { task: { factory: "f1", task: "T-1" } });
  await act(async () => useShellStore.setState({ factoryTask: { factory: "f1", task: "T-1", detail: detail(state, [...allowed]) } }));
  expect(container.querySelector("[data-factory-actions]")!.getAttribute("data-factory-actions")).toBe(drawn);
  expect(events.some((event) => (event as unknown as { kind: string }).kind === "factory_task_open")).toBe(true);
});

it("draws no action for a blocked Task, which takes only an answer (B19)", async () => {
  const { container } = await mount({ my_turn: 0, factories: [factory()], inbox: [] }, { task: { factory: "f1", task: "T-1" } });
  await act(async () => useShellStore.setState({ factoryTask: { factory: "f1", task: "T-1", detail: detail("blocked", []) } }));
  expect(container.querySelector("[data-factory-task-state]")).not.toBeNull();
  expect(container.querySelector("[data-factory-actions]")).toBeNull();
});

const CONFIG = {
  verification: { kind: "ci", checks: ["ci"] }, merge_mode: "manual", merge_method: "merge", quick_check: null, question_deadline_ms: 86_400_000, stall_ms: 1_800_000,
  no_report_ms: 3_600_000, watch_interval_ms: 1_800_000, watch_daily_limit: 4, outside_read_ms: 120_000, cancel_keep_ms: 604_800_000, done_fold_ms: 259_200_000,
  archive_fold_ms: 7_776_000_000, new_task_limit: 10, verify_failure_limit: 3, verify_timeout_ms: 3_600_000, disk_floor_bytes: 10_737_418_240, default_runtime: "claude",
  harness: null, autonomy: [], autonomy_diff_limit: 400, recovery: [], risk_paths: [], checks: [], prd_in_issue: false, macos_notifications: false, worker_args: {},
  workers: [], observer_mode: "assist", observer_daily_limit: 100, factory_ai: null,
};

/** Answers the settings tab's config read the way the engine does. */
async function answerConfig(summary: FactorySummary, events: Parameters<DispatchFn>[0][], config: object = CONFIG) {
  const read = events.find((event) => (event as unknown as { kind: string; payload: { command: { verb: string } } }).payload?.command?.verb === "config") as unknown as { payload: { request_id: string } };
  await act(async () => useShellStore.setState({ factory: { summary, actions: [{ request_id: read.payload.request_id, answer: { ok: true, config, machine: { max_workers: 5 } } }] } }));
}

it("keeps Close disabled while the Running column holds a Task in any of its states, as the engine refuses then (B22)", async () => {
  const waiting = factory({ columns: [{ column: "moving", label: "running", cards: [card("T-1", "merge_waiting", { column: "moving" })] }] });
  const summary = { my_turn: 0, factories: [waiting], inbox: [] };
  const { container, events } = await mount(summary, { tab: "settings", factory: "f1" });
  await answerConfig(summary, events);
  expect(container.querySelector<HTMLButtonElement>("[data-factory-close]")!.disabled).toBe(true);
  const done = { my_turn: 0, factories: [factory({ columns: [{ column: "done", label: "done", cards: [card("T-1", "done", { column: "done" })] }] })], inbox: [] };
  await answerConfig(done, events);
  expect(container.querySelector<HTMLButtonElement>("[data-factory-close]")!.disabled).toBe(false);
});

it("takes an answer that comes after the wait ran out, since the engine may still finish the work (B10)", async () => {
  vi.useFakeTimers();
  try {
    const { container, events } = await mount({ my_turn: 1, factories: [factory()], inbox: [MERGE] });
    await act(async () => container.querySelector<HTMLButtonElement>("[data-factory-send]")!.click());
    const sent = events.at(-1) as unknown as { payload: { request_id: string } };
    await act(async () => vi.advanceTimersByTime(REQUEST_ANSWER_TIMEOUT_MS + 1));
    expect(container.querySelector("[data-factory-refused]")!.getAttribute("data-factory-refused")).toBe("no_answer");
    await act(async () => useShellStore.setState({ factory: { summary: { my_turn: 1, factories: [factory()], inbox: [MERGE] }, actions: [{ request_id: sent.payload.request_id, answer: { ok: true } }] } }));
    expect(container.querySelector("[data-factory-refused]")).toBeNull();
    expect(container.querySelector("[data-factory-send]")!.getAttribute("data-factory-send")).toBe("taken");
  } finally {
    vi.useRealTimers();
  }
});

it("says the engine's choices in words, and opens Hide AI in Settings while it is off, answering once it is on (B5, B22)", async () => {
  const failed: InboxItem = { ...MERGE, group: "stopped", kind: "action", question: "Q-1", text: "The intake review could not run because Hide AI is off.", holding: "start", suggestion: "enable-ai", choices: ["start-as-is"], result_code: "drafting", gates: [] };
  await act(async () => useShellStore.setState((state) => ({ rest: { ...state.rest, status: { ...state.rest?.status, background_ai: { enabled: false, provider: null, chosen: false, providers: [], unavailable_reason: null } } } as never })));
  const { container, events } = await mount({ my_turn: 1, factories: [factory()], inbox: [failed] });
  const choices = [...container.querySelectorAll("[data-factory-choice]")].map((choice) => choice.querySelector("span:nth-child(2) > span")!.textContent);
  expect(choices).toEqual([english["factory.choice.enableAi"], english["factory.choice.startAsIs"]]);
  const send = () => container.querySelector<HTMLButtonElement>("[data-factory-send]")!;
  expect(send().textContent).toBe(english["factory.choice.openAi"]);
  await act(async () => container.querySelector<HTMLButtonElement>("[data-factory-choice='2']")!.click());
  expect(send().textContent).toBe(english["factory.decide.send"]);
  await act(async () => container.querySelector<HTMLButtonElement>("[data-factory-choice='1']")!.click());
  const before = events.length;
  await act(async () => send().click());
  expect(events.length).toBe(before);
  expect(useUiStore.getState()).toMatchObject({ overlay: "settings", settingsTab: "hideAi" });
  // Turning it on sends the answer the person already gave, once.
  await act(async () => useShellStore.setState((state) => ({ rest: { ...state.rest, status: { ...state.rest?.status, background_ai: { enabled: true, provider: "claude", chosen: true, providers: [], unavailable_reason: null } } } as never })));
  expect(lastAction(events).payload.command).toMatchObject({ verb: "answer", task: "f1/T-1", question: "Q-1", choice: "suggestion" });
  expect(events.length).toBe(before + 1);
});

it("offers a to-do's command to copy and one button that resolves it in its Factory (B16, B23)", async () => {
  const todo: InboxItem = { ...MERGE, group: "todo", kind: "command", task: null, display_id: null, text: "Close the old terminal holding the worker's name", holding: "starts", command: "herdr pane close w4:p2", impact: "Starts the worker again", resolve: "C1", result_code: "resolve", choices: [], suggestion: "", gates: [] };
  const { container, events } = await mount({ my_turn: 1, factories: [factory()], inbox: [todo] });
  expect(container.querySelector("[data-factory-command] code")!.textContent).toBe("herdr pane close w4:p2");
  expect(container.querySelectorAll("[data-factory-choice]")).toHaveLength(0);
  expect(container.querySelector("[data-factory-result='command']")!.textContent).toBe("Starts the worker again");
  await act(async () => container.querySelector<HTMLButtonElement>("[data-factory-send]")!.click());
  expect(lastAction(events).payload.command).toEqual({ verb: "resolve", project: "/fixture", item: "C1" });
});

it("shows where a project check failed and the engine's next action, a logged-out gh's included (B5)", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  const state = { phase: "refused", answer: { ok: false, reason: "init_failed", next_action: "Run gh auth login, then retry", detail: { stage: "probe" } } } as const;
  await act(async () => root!.render(<InitFailure state={state} />));
  expect(container.querySelector("[data-factory-create-stage]")!.textContent).toBe(english["factory.create.failedStage"].replace("{{stage}}", english["factory.create.stage.probe"]));
  expect(container.querySelector("[data-factory-create-next]")!.textContent).toContain("gh auth login");
});

it("offers a Factory on this machine and on a node that dials the core, never on a device the core dials (Q17)", () => {
  const device = (id: string, kind: string, dials_in: boolean) => ({ id, kind, dials_in });
  const rest = { navigator: { devices: [device("core-node", "local", false), device("macbook-node", "remote", true), device("studio", "remote", false)] } } as unknown as SnapshotRest;
  expect([...factoryMachines(rest)].sort()).toEqual(["core-node", "macbook-node"]);
});

function sentVerbs(events: Parameters<DispatchFn>[0][]): string[] {
  return events.flatMap((event) => {
    const sent = event as unknown as { kind: string; payload: { command: { verb: string } } };
    return sent.kind === "factory_action" ? [sent.payload.command.verb] : [];
  });
}

function press(target: Element, init: KeyboardEventInit) {
  target.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
}

it("sends the picked answer on Enter, but leaves Enter on the Task's id to open the Task and Enter mid-composition alone (B9)", async () => {
  const { container, events } = await mount({ my_turn: 1, factories: [factory()], inbox: [MERGE] });
  const details = container.querySelector<HTMLButtonElement>("[data-factory-item-id]")!;
  await act(async () => press(details, { key: "Enter" }));
  expect(sentVerbs(events)).toEqual([]);
  const choice = container.querySelector("[data-factory-choice='1']")!;
  await act(async () => press(choice, { key: "Enter", isComposing: true }));
  expect(sentVerbs(events)).toEqual([]);
  await act(async () => press(choice, { key: "Enter" }));
  expect(sentVerbs(events)).toEqual(["merge"]);
});

it("sends a setting once though Enter and leaving the field both commit, and shows the saved value again when the engine refuses it (B22)", async () => {
  const summary = { my_turn: 0, factories: [factory()], inbox: [] };
  const { container, events } = await mount(summary, { tab: "settings", factory: "f1" });
  await answerConfig(summary, events);
  const field = container.querySelector<HTMLInputElement>("[data-factory-setting='watch_daily_limit']")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(field, "0");
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => press(field, { key: "Enter" }));
  await act(async () => field.dispatchEvent(new FocusEvent("focusout", { bubbles: true })));
  const writes = events.filter((event) => (event as unknown as { payload: { command: { set?: unknown[] } } }).payload?.command?.set?.length);
  expect(writes).toHaveLength(1);
  const write = writes[0] as unknown as { payload: { request_id: string } };
  await act(async () => useShellStore.setState({ factory: { summary, actions: [{ request_id: write.payload.request_id, answer: { ok: false, reason: "config_invalid", next_action: "Check the value" } }] } }));
  expect(container.querySelector<HTMLInputElement>("[data-factory-setting='watch_daily_limit']")!.value).toBe(String(CONFIG.watch_daily_limit));
});

it("keeps a check's instruction until the engine takes it, then reads the config again, since a check answers with no config (B22)", async () => {
  const summary = { my_turn: 0, factories: [factory()], inbox: [] };
  const { container, events } = await mount(summary, { tab: "settings", factory: "f1" });
  await answerConfig(summary, events);
  const field = container.querySelector<HTMLInputElement>("[data-factory-setting='check_instruction']")!;
  await act(async () => type(field, "README가 최신인지"));
  const add = [...container.querySelectorAll("button")].find((button) => button.textContent === english["factory.settings.add"])!;
  await act(async () => add.click());
  const check = lastAction(events);
  expect(check.payload.command).toMatchObject({ verb: "check", instruction: "README가 최신인지" });
  await act(async () => useShellStore.setState({ factory: { summary, actions: [{ request_id: check.payload.request_id, answer: { ok: false, reason: "instruction_required", next_action: "Give --instruction" } }] } }));
  expect(container.querySelector<HTMLInputElement>("[data-factory-setting='check_instruction']")!.value).toBe("README가 최신인지");
  await act(async () => add.click());
  const again = lastAction(events);
  await act(async () => useShellStore.setState({ factory: { summary, actions: [{ request_id: again.payload.request_id, answer: { ok: true, message: "check added" } }] } }));
  expect(container.querySelector<HTMLInputElement>("[data-factory-setting='check_instruction']")!.value).toBe("");
  expect(lastAction(events).payload.command).toEqual({ verb: "config", project: "/fixture", set: [] });
  expect(container.querySelector("[data-factory-close]")).not.toBeNull();
});

it("holds Add while a check is on its way, and keeps a refused check's text through a later write of another setting (B22)", async () => {
  const summary = { my_turn: 0, factories: [factory()], inbox: [] };
  const { container, events } = await mount(summary, { tab: "settings", factory: "f1" });
  await answerConfig(summary, events);
  await act(async () => type(container.querySelector<HTMLInputElement>("[data-factory-setting='check_instruction']")!, "README"));
  const add = [...container.querySelectorAll("button")].find((button) => button.textContent === english["factory.settings.add"])!;
  await act(async () => add.click());
  expect(add.disabled).toBe(true);
  const check = lastAction(events);
  await act(async () => useShellStore.setState({ factory: { summary, actions: [{ request_id: check.payload.request_id, answer: { ok: false, reason: "factory_busy", next_action: "Try again in a moment" } }] } }));
  const field = container.querySelector<HTMLInputElement>("[data-factory-setting='new_task_limit']")!;
  await act(async () => type(field, "7"));
  await act(async () => press(field, { key: "Enter" }));
  const write = lastAction(events);
  await act(async () => useShellStore.setState({ factory: { summary, actions: [{ request_id: write.payload.request_id, answer: { ok: true, config: { ...CONFIG, new_task_limit: 7 }, machine: { max_workers: 5 } } }] } }));
  expect(container.querySelector<HTMLInputElement>("[data-factory-setting='check_instruction']")!.value).toBe("README");
});

it("keeps a control for every per-Factory value hide factory config sets, the rest of them under 고급 설정 (B36)", async () => {
  const summary = { my_turn: 0, factories: [factory()], inbox: [] };
  const { container, events } = await mount(summary, { tab: "settings", factory: "f1" });
  await answerConfig(summary, events);
  // docs/factory.md, Configuration: the keys a person sets for one Factory; CONFIG verifies through CI, so its field is `ci`.
  const keys = ["merge_mode", "merge_method", "ci", "quick_check", "question_deadline_hours", "stall_minutes", "no_report_minutes", "watch_interval_minutes", "watch_daily_limit", "outside_read_minutes", "cancel_keep_days", "done_fold_days", "archive_fold_days", "new_task_limit", "verify_failure_limit", "autonomy_diff_limit", "verify_timeout_minutes", "disk_floor_gb", "harness", "risk_paths", "prd_in_issue", "macos_notifications", "observer_mode", "observer_daily_limit", "factory_ai", "check_instruction", "worker_args:claude", "worker_args:codex"];
  expect(keys.filter((key) => !container.querySelector(`[data-factory-setting='${key}']`))).toEqual([]);
  expect([...container.querySelectorAll("[data-factory-setting^='recovery:']")]).toHaveLength(5);
  const advanced = container.querySelector("[data-factory-settings-group='advanced']")!;
  expect([...advanced.querySelectorAll("[data-factory-settings-subhead]")].map((node) => node.getAttribute("data-factory-settings-subhead"))).toEqual(["verification", "merge", "run", "checks", "autonomy"]);
});

it("names a card's worker by the agent's name the engine gives, not its id (B43)", async () => {
  const running = factory({ columns: [{ column: "moving", label: "moving", cards: [card("T-1", "running", { column: "moving", worker_runtime: "claude", worker_label: "Claude Code" })] }] });
  const { container } = await mount({ my_turn: 0, factories: [running], inbox: [] }, { tab: "board" });
  const mark = container.querySelector(".factory-card-logo")!;
  expect(mark.getAttribute("aria-label")).toBe("Claude Code");
  expect(mark.getAttribute("title")).toBe("Claude Code");
});

it("treats a Factory whose cards are all archived as empty, not as a filter that matches nothing (B15)", async () => {
  const archived = factory({ columns: [{ column: "done", label: "done", cards: [card("T-1", "done", { column: "done", archived: true, folded: true })] }] });
  const { container } = await mount({ my_turn: 0, factories: [archived], inbox: [] }, { tab: "board" });
  expect(container.querySelector("[data-factory-board-empty='intake']")).not.toBeNull();
});

it("waits behind Settings or a close confirmation instead of swapping the screen under it (B2)", () => {
  const actions = createActions(() => true);
  useUiStore.setState({ screen: { kind: "main" }, overlay: "settings", pendingClose: null });
  actions.openFactory();
  expect(useUiStore.getState().screen?.kind).toBe("main");
  useUiStore.setState({ overlay: "none", pendingClose: { paneId: "p1" } as never });
  actions.openFactory();
  expect(useUiStore.getState().screen?.kind).toBe("main");
  useUiStore.setState({ pendingClose: null });
  actions.openFactory();
  expect(useUiStore.getState().screen?.kind).toBe("factory");
});

function type(field: HTMLInputElement, value: string) {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(field, value);
  field.dispatchEvent(new Event("input", { bubbles: true }));
}

function lastAction(events: Parameters<DispatchFn>[0][]) {
  return events.filter((event) => (event as unknown as { kind: string }).kind === "factory_action").at(-1) as unknown as { payload: { request_id: string; command: Record<string, unknown> } };
}

it("puts an emptied number back to the saved value instead of sending 0 (B22)", async () => {
  const summary = { my_turn: 0, factories: [factory()], inbox: [] };
  const { container, events } = await mount(summary, { tab: "settings", factory: "f1" });
  await answerConfig(summary, events);
  const field = container.querySelector<HTMLInputElement>("[data-factory-setting='question_deadline_hours']")!;
  await act(async () => type(field, ""));
  await act(async () => field.dispatchEvent(new FocusEvent("focusout", { bubbles: true })));
  expect(events.some((event) => ((event as unknown as { payload: { command?: { set?: unknown[] } } }).payload?.command?.set?.length ?? 0) > 0)).toBe(false);
  expect(container.querySelector<HTMLInputElement>("[data-factory-setting='question_deadline_hours']")!.value).toBe("24");
});

it("keeps a change request's comment while the engine refuses it, and closes the form once taken (B19)", async () => {
  const { container, events } = await mount({ my_turn: 0, factories: [factory()], inbox: [] }, { task: { factory: "f1", task: "T-1" } });
  await act(async () => useShellStore.setState({ factoryTask: { factory: "f1", task: "T-1", detail: detail("merge_waiting", ["merge", "request-changes", "cancel"]) } }));
  await act(async () => container.querySelector<HTMLButtonElement>("[data-factory-action='request-changes']")!.click());
  await act(async () => type(container.querySelector<HTMLInputElement>("[data-factory-comment]")!, "테스트를 더 써 주세요"));
  await act(async () => container.querySelector("[data-factory-comment]")!.closest("form")!.requestSubmit());
  const sent = lastAction(events);
  expect(sent.payload.command).toMatchObject({ verb: "request_changes", comment: "테스트를 더 써 주세요" });
  const summary = { my_turn: 0, factories: [factory()], inbox: [] };
  await act(async () => useShellStore.setState({ factory: { summary, actions: [{ request_id: sent.payload.request_id, answer: { ok: false, reason: "factory_busy", next_action: "Try again in a moment" } }] } }));
  expect(container.querySelector<HTMLInputElement>("[data-factory-comment]")!.value).toBe("테스트를 더 써 주세요");
  await act(async () => container.querySelector("[data-factory-comment]")!.closest("form")!.requestSubmit());
  const again = lastAction(events);
  await act(async () => useShellStore.setState({ factory: { summary, actions: [{ request_id: again.payload.request_id, answer: { ok: true } }] } }));
  expect(container.querySelector("[data-factory-comment]")).toBeNull();
});

it("says a Task is gone when its Factory has left the summary, instead of loading forever", async () => {
  const { container } = await mount({ my_turn: 0, factories: [factory()], inbox: [] }, { task: { factory: "gone", task: "T-1" } });
  expect(container.querySelector("[data-factory-task-missing]")).not.toBeNull();
  expect(container.querySelector("[data-factory-task-loading]")).toBeNull();
});

it("shows a Task page as reading until its section comes, and one that could not be read with a button that reads it again (B32)", async () => {
  const { container, events } = await mount({ my_turn: 0, factories: [factory()], inbox: [] }, { task: { factory: "f1", task: "T-1" } });
  await act(async () => useShellStore.setState({ factoryTask: null }));
  expect(container.querySelector("[data-factory-task-loading]")).not.toBeNull();
  await act(async () => useShellStore.setState({ factoryTask: { factory: "f1", task: "T-1", detail: null } }));
  expect(container.querySelector("[data-factory-task-loading]")).toBeNull();
  expect(container.textContent).toContain(english["factory.task.unreadable"]);
  const before = events.length;
  await act(async () => container.querySelector<HTMLButtonElement>("[data-factory-task-reread]")!.click());
  expect(events.slice(before).map((event) => (event as unknown as { kind: string; payload: unknown }).kind)).toEqual(["factory_task_close", "factory_task_open"]);
  expect((events.at(-1) as unknown as { payload: unknown }).payload).toEqual({ factory: "f1", task: "T-1" });
});

it("does not send a taken answer again while its item is still on screen (B10)", async () => {
  const { container, events } = await mount({ my_turn: 1, factories: [factory()], inbox: [MERGE] });
  const send = container.querySelector<HTMLButtonElement>("[data-factory-send]")!;
  await act(async () => send.click());
  const sent = lastAction(events);
  await act(async () => useShellStore.setState({ factory: { summary: { my_turn: 1, factories: [factory()], inbox: [MERGE] }, actions: [{ request_id: sent.payload.request_id, answer: { ok: true } }] } }));
  expect(container.querySelector<HTMLButtonElement>("[data-factory-send]")!.disabled).toBe(true);
  await act(async () => press(container.querySelector("[data-factory-choice='1']")!, { key: "Enter" }));
  expect(sentVerbs(events)).toEqual(["merge"]);
});

it("puts a number the field cannot take back to the saved value instead of leaving it as if saved (B22)", async () => {
  const summary = { my_turn: 0, factories: [factory()], inbox: [] };
  const { container, events } = await mount(summary, { tab: "settings", factory: "f1" });
  await answerConfig(summary, events);
  const field = container.querySelector<HTMLInputElement>("[data-factory-setting='watch_daily_limit']")!;
  await act(async () => type(field, "2.5"));
  await act(async () => field.dispatchEvent(new FocusEvent("focusout", { bubbles: true })));
  expect(container.querySelector<HTMLInputElement>("[data-factory-setting='watch_daily_limit']")!.value).toBe(String(CONFIG.watch_daily_limit));
});

it("leaves an open priority form alone when another action on the page is taken (B19)", async () => {
  const { container, events } = await mount({ my_turn: 0, factories: [factory()], inbox: [] }, { task: { factory: "f1", task: "T-1" } });
  await act(async () => useShellStore.setState({ factoryTask: { factory: "f1", task: "T-1", detail: detail("waiting", ["priority", "cancel"]) } }));
  await act(async () => container.querySelector<HTMLButtonElement>("[data-factory-action='priority']")!.click());
  await act(async () => container.querySelector<HTMLButtonElement>("[data-factory-action='cancel']")!.click());
  const cancel = lastAction(events);
  await act(async () => useShellStore.setState({ factory: { summary: { my_turn: 0, factories: [factory()], inbox: [] }, actions: [{ request_id: cancel.payload.request_id, answer: { ok: true } }] } }));
  expect(container.querySelector("[data-factory-priority]")).not.toBeNull();
});

it("shows each criterion with what the last check said of it, and cues a shortened decision (B27, B19)", async () => {
  const { container } = await mount({ my_turn: 0, factories: [factory()], inbox: [] }, { task: { factory: "f1", task: "T-1" } });
  const done: TaskDetail = {
    ...detail("done", []),
    checklist: [{ text: "README가 최신", state: "met", reason: null }, { text: "색인이 200 MB 이하", state: "unmet", reason: "240 MB" }],
    decisions: [decision({ id: "R1", by: "worker", text: "is al [cut 15 bytes] -> 머지 결정" })],
  };
  await act(async () => useShellStore.setState({ factoryTask: { factory: "f1", task: "T-1", detail: done } }));
  expect([...container.querySelectorAll("[data-factory-criterion]")].map((row) => row.getAttribute("data-factory-criterion"))).toEqual(["met", "unmet"]);
  expect(container.querySelector("[data-factory-criterion='unmet']")!.textContent).toContain("240 MB");
  const decisions = container.querySelector("[data-factory-decisions]")!;
  expect(decisions.textContent).not.toContain("[cut");
  expect(decisions.querySelector("[data-factory-cut]")).not.toBeNull();
});

it("reads the goal as the Markdown it is written in, keeping its lines", async () => {
  const { container } = await mount({ my_turn: 0, factories: [factory()], inbox: [] }, { task: { factory: "f1", task: "T-1" } });
  const goal = "## What hide does today\n\nPi closes its pane.\n\n## What it should do instead\n\n- wake in a new pane\n- keep its letters";
  await act(async () => useShellStore.setState({ factoryTask: { factory: "f1", task: "T-1", detail: { ...detail("running", []), goal } } }));
  const shown = container.querySelector("[data-factory-goal] [data-markdown-text]")!;
  expect(shown).not.toBeNull();
  const lines = [...shown.querySelectorAll(".cm-line")].map((line) => line.textContent);
  expect(lines).toContain("Pi closes its pane.");
  expect(lines.filter((line) => line?.includes("keep its letters"))).toHaveLength(1);
  expect(lines.length).toBeGreaterThanOrEqual(8);
});

it("opens a decision whose question the engine shortened to the whole question in place", async () => {
  const { container } = await mount({ my_turn: 0, factories: [factory()], inbox: [] }, { task: { factory: "f1", task: "T-1" } });
  const whole = "A session path over 256 bytes keeps a Pi worker from starting, so should the Factory keep retrying it for ever";
  const question: TaskDetail["questions"][number] = { id: "Q17", origin: "worker", kind: { kind: "default" }, text: whole, suggestion: "", default_action: null, deadline: null, asked_at: NOW, choices: ["ok"], answer: { text: "ok", chose: "ok", relayed_by: "screen", at: NOW }, letter: null };
  const page: TaskDetail = { ...detail("running", []), questions: [question], decisions: [decision({ id: "R1", by: "person", text: `${whole.slice(0, 40)}\n[cut 80 bytes] -> ok` })] };
  await act(async () => useShellStore.setState({ factoryTask: { factory: "f1", task: "T-1", detail: page } }));
  const row = container.querySelector("[data-factory-decisions]")!;
  expect(row.textContent).not.toContain("for ever");
  expect(row.querySelector("[data-factory-cut]")).not.toBeNull();
  await act(async () => row.querySelector<HTMLButtonElement>("[data-factory-decision-whole]")!.click());
  expect(row.querySelector("[data-factory-decision] .text-caption")!.textContent).toBe(whole);
  expect(row.querySelector("[data-factory-cut]")).toBeNull();
  await act(async () => row.querySelector<HTMLButtonElement>("[data-factory-decision-whole]")!.click());
  expect(row.textContent).not.toContain("for ever");
});

it("draws the Task's activity with a failed check's log and a report's unchecked part in the warning tone (B30)", async () => {
  const { container } = await mount({ my_turn: 0, factories: [factory()], inbox: [] }, { task: { factory: "f1", task: "T-1" } });
  const page: TaskDetail = {
    ...detail("verifying", []),
    attempts: [{ number: 1, stage: "task", started_at: NOW, outcome: "failed", check: "web-e2e", link: null, log_tail: "1 failed" }],
    activity: [
      { at: NOW - 2, task: "T-1", kind: "report", report: { result: "Search answers from the index", changed: ["index"], verified: ["cargo test"], unverified: ["10k sessions"] } },
      { at: NOW - 1, task: "T-1", kind: "verification", number: 1, ci: false, outcome: "failed", check: "web-e2e" },
    ],
  };
  await act(async () => useShellStore.setState({ factoryTask: { factory: "f1", task: "T-1", detail: page } }));
  expect([...container.querySelectorAll("[data-factory-activity]")].map((row) => row.getAttribute("data-factory-activity"))).toEqual(["report", "verification", "now"]);
  const unchecked = [...container.querySelectorAll("[data-factory-activity='report'] .text-warning")].map((node) => node.textContent);
  expect(unchecked).toContain("10k sessions");
  expect(container.querySelector("[data-factory-activity='verification']")!.textContent).toContain(english["factory.task.showLog"]);
});

it("keeps the graph in columns and logs why when the layout worker cannot start (D-08)", async () => {
  // jsdom has no Worker, so the layered layout refuses at once, the way a failed worker does.
  const graphed = factory({
    columns: [{ column: "before", label: "waiting", cards: [card("T-1", "waiting", { column: "before" }), card("T-2", "waiting", { column: "before" })] }],
    graph: { nodes: ["T-1", "T-2"], edges: [["T-1", "T-2"]], unrelated: [] },
    dependencies: [["T-1", "T-2"]],
  });
  useShellStore.setState({ diagnostics: [] });
  // jsdom has no CSS.escape; the ids here need no escaping.
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  const { container } = await mount({ my_turn: 0, factories: [graphed], inbox: [] }, { tab: "graph" });
  await act(async () => {
    await vi.waitFor(() => expect(useShellStore.getState().diagnostics.some((line) => line.includes("dependency graph stays in columns"))).toBe(true));
  });
  expect(container.querySelector("[data-dependency-graph]")!.getAttribute("data-dependency-layout")).toBe("columns");
  expect(container.querySelectorAll("[data-dependency-layer]")).toHaveLength(2);
});

// These fail if a card sends another command, navigates on an action, or forgets its taken request.
it.each([
  ["blocked", { ...MERGE, group: "answer", kind: "blocking", question: "q1", suggestion: "WS", choices: ["REST"], text: "Which endpoint?" }, { verb: "answer", task: "f1/T-1", question: "q1", choice: "suggestion", text: null }],
  ["merge_waiting", MERGE, { verb: "merge", task: "f1/T-1" }],
  ["stopped", { ...MERGE, group: "stopped", kind: "stopped", suggestion: "retry", stop: "verify_failed" }, { verb: "retry", task: "f1/T-1" }],
  ["stopped", { ...MERGE, group: "stopped", kind: "action", question: "q1", suggestion: "retry", stop: "verify_failed" }, { verb: "retry", task: "f1/T-1" }],
  ["stopped", { ...MERGE, group: "answer", kind: "new_task_cap", question: "q1", suggestion: "raise cap", stop: "verify_failed" }, { verb: "retry", task: "f1/T-1" }],
] as const)("sends the %s card's canonical command once, preserves a refusal and allows retry", async (state, item, expected) => {
  const task = card("T-1", state, { column: "stuck", waiting_group: "person", needs_person: true, stop: state === "stopped" ? "verify_failed" : null });
  const summary = { my_turn: 1, factories: [factory({ columns: [{ column: "stuck", label: "", cards: [task] }] })], inbox: [{ ...item, choices: [...item.choices], gates: [...item.gates], unblocks: [...item.unblocks] } as InboxItem] };
  if (state === "merge_waiting" || item.kind === "stopped") summary.inbox.unshift({ ...MERGE, kind: "action", question: "older-question", suggestion: "approve", gates: [] });
  const { container, events } = await mount(summary, { tab: "board" });
  const send = container.querySelector<HTMLButtonElement>("[data-factory-card-send]")!;
  expect(send).not.toBeNull();
  await act(async () => send.click());
  expect((events.at(-1)!.payload as { command: unknown }).command).toEqual(expected);
  expect(useUiStore.getState().screen).toMatchObject({ place: { task: null } });
  expect(send.disabled).toBe(true);
  const sent = events.at(-1)!;
  await act(async () => send.click());
  expect(events.at(-1)).toBe(sent);
  const request = sent.payload as { request_id: string };
  await act(async () => useShellStore.setState({ factory: { summary, actions: [{ request_id: request.request_id, answer: { ok: false, reason: "action_not_allowed_in_state", next_action: "Read current task" } }] } }));
  expect(container.querySelector("[data-factory-refused]")).not.toBeNull();
  expect(send.disabled).toBe(false);
  await act(async () => send.click());
  const retry = events.at(-1)!.payload as { request_id: string };
  await act(async () => useShellStore.setState({ factory: { summary, actions: [{ request_id: retry.request_id, answer: { ok: true } }] } }));
  expect(send.disabled).toBe(true);
});

it("opens the corresponding inbox item for another answer and keeps issue/PR controls separate from card navigation", async () => {
  const task = card("T-1", "blocked", { column: "stuck", waiting_group: "person", needs_person: true, issue: "#12", issue_url: "https://example.invalid/issues/12", pr: { number: 34, url: "https://example.invalid/pull/34", head: "change", by_factory: true, open: true } });
  const item: InboxItem = { ...MERGE, group: "answer", kind: "blocking", question: "q1", suggestion: "WS", choices: ["REST"] };
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  const { container } = await mount({ my_turn: 1, factories: [factory({ columns: [{ column: "stuck", label: "", cards: [task] }] })], inbox: [item] }, { tab: "board" });
  expect(container.querySelector("[data-factory-card] a")!.getAttribute("href")).toBe(task.issue_url);
  expect(container.querySelectorAll("button button").length).toBe(0);
  const other = [...container.querySelectorAll<HTMLButtonElement>("[data-factory-card] button")].find((button) => button.textContent === english["factory.card.otherAnswer"])!;
  await act(async () => other.click());
  expect(document.activeElement!.closest("[data-factory-item]")!.getAttribute("data-factory-item")).toBe("f1/T-1/q1");
  expect(useUiStore.getState().screen).toMatchObject({ place: { tab: "line", task: null, focus: null } });
});

it("opens a local issue through its catalog identity without opening the Factory task", async () => {
  const rest = { navigator: { focused_device_id: "remote", devices: [{ id: "remote", kind: "remote" }, { id: "local", kind: "local" }], workspaces: [
    { id: "remote-project", path: "/fixture", device_id: "remote", checkouts: [], agent_scope: {}, tasks: { tasks: [{ id: "L-7", key: "remote:/fixture#7", source: "local" }] } },
    { id: "project", path: "/fixture", device_id: "local", checkouts: [], agent_scope: {}, tasks: { tasks: [{ id: "L-7", key: "local:/fixture#7", source: "local" }] } },
  ] } } as unknown as SnapshotRest;
  useShellStore.setState({ rest });
  const task = card("T-1", "running", { issue: "L-7" });
  const { container } = await mount({ my_turn: 0, factories: [factory({ columns: [{ column: "moving", label: "", cards: [task] }] })], inbox: [] }, { tab: "board" });
  const issue = [...container.querySelectorAll<HTMLButtonElement>("[data-factory-card] button")].find((button) => button.textContent === "L-7")!;
  await act(async () => issue.click());
  expect(useUiStore.getState().mainView).toBe("backlog");
  expect(useUiStore.getState().overview.panel).toEqual({ kind: "item", workspaceId: "project", taskKey: "local:/fixture#7" });
  expect(useUiStore.getState().screen).toMatchObject({ kind: "main", deviceId: "local" });
  useShellStore.setState({ rest: null });
});

function commands(events: Parameters<DispatchFn>[0][]): Record<string, unknown>[] {
  return events.flatMap((event) => {
    const sent = event as unknown as { kind: string; payload: { command: Record<string, unknown> } };
    return sent.kind === "factory_action" ? [sent.payload.command] : [];
  });
}

it("says why a request Factory AI sorted is still the person's in the Factory's mode (D-14, D-21)", async () => {
  const question: InboxItem = { ...MERGE, group: "answer", kind: "blocking", question: "q-2", text: "빈 열에 무엇을?", suggestion: "없음", choices: ["안내"], result_code: "wake_worker", gates: [], decision_kind: "C", observer_reason: "작업자는 첫 안을 추천합니다." };
  const { container } = await mount({ my_turn: 1, factories: [factory()], inbox: [question] });
  await act(async () => container.querySelector<HTMLButtonElement>("[data-factory-evidence] button")!.click());
  const rows = [...container.querySelectorAll("[data-factory-evidence] dd")].map((row) => row.textContent);
  expect(rows).toEqual([`${english["factory.decision.C"]} · ${english["factory.decision.mine.assist"]}`, "작업자는 첫 안을 추천합니다."]);
});

it("pauses the one Factory the header shows, and says it is paused with a way to resume (D-48)", async () => {
  const { container, events } = await mount({ my_turn: 0, factories: [factory()], inbox: [] }, { factory: "f1" });
  await act(async () => container.querySelector<HTMLButtonElement>("[data-factory-pause='pause']")!.click());
  expect(commands(events)).toEqual([{ verb: "pause_factory", project: "/fixture" }]);
  const paused = lastAction(events).payload.request_id;
  await act(async () => useShellStore.setState({ factory: { summary: { my_turn: 0, factories: [factory({ paused: true })], inbox: [] }, actions: [{ request_id: paused, answer: { ok: true } }] } }));
  expect(container.querySelector("[data-factory-paused-chip]")).not.toBeNull();
  await act(async () => container.querySelector<HTMLButtonElement>("[data-factory-pause='resume']")!.click());
  expect(commands(events).at(-1)).toEqual({ verb: "resume_factory", project: "/fixture" });
});

it("lists every Factory when all projects are shown, and pauses one from its row (B36)", async () => {
  const other = factory({ id: "f2", project: "/other", project_name: "other", paused: true, observer_mode: "autonomous" });
  const { container, events } = await mount({ my_turn: 0, factories: [factory(), other], inbox: [] }, { tab: "settings" });
  expect([...container.querySelectorAll("[data-factory-list-row]")].map((row) => row.getAttribute("data-factory-list-row"))).toEqual(["f1", "f2"]);
  expect(container.querySelector("[data-factory-list-row='f2'] [data-factory-list-paused]")!.getAttribute("data-factory-list-paused")).toBe("true");
  await act(async () => container.querySelector<HTMLButtonElement>("[data-factory-list-pause='f2']")!.click());
  expect(commands(events).at(-1)).toEqual({ verb: "resume_factory", project: "/other" });
  await act(async () => container.querySelector<HTMLButtonElement>("[data-factory-list-open='f2']")!.click());
  expect(useUiStore.getState().screen).toMatchObject({ place: { factory: "f2" } });
});

it("reads a Factory made before worker candidates as one of its default agent, and sends the whole list on a change (D-41, D-42)", async () => {
  const summary = { my_turn: 0, factories: [factory()], inbox: [] };
  const { container, events } = await mount(summary, { tab: "settings", factory: "f1" });
  await answerConfig(summary, events);
  expect(container.querySelectorAll("[data-factory-worker-candidate]")).toHaveLength(1);
  await act(async () => container.querySelector<HTMLButtonElement>("[data-factory-worker-add]")!.click());
  expect(lastAction(events).payload.command).toEqual({ verb: "config", project: "/fixture", set: [["workers", JSON.stringify([{ agent: "claude", description: "" }, { agent: "claude", description: "" }])]] });
});

it("offers a model menu only for a worker whose agent's start takes a model (B28, D-42)", async () => {
  const summary = { my_turn: 0, factories: [factory()], inbox: [] };
  const { container, events } = await mount(summary, { tab: "settings", factory: "f1" });
  await answerConfig(summary, events, { ...CONFIG, workers: [{ agent: "claude", description: "" }, { agent: "grok", description: "" }, { agent: "opencode", description: "" }] });
  const row = (at: number) => container.querySelector(`[data-factory-worker-candidate='${at}']`)!;
  expect(row(1).querySelector("[data-agent-model]")).not.toBeNull();
  expect(row(2).querySelector("[data-agent-kind='grok']")).not.toBeNull();
  expect(row(2).querySelector("[data-agent-model]")).not.toBeNull();
  expect(row(3).querySelector("[data-agent-kind='opencode']")).not.toBeNull();
  expect(row(3).querySelector("[data-agent-model]")).toBeNull();
  expect(row(3).textContent).toContain(english["factory.settings.cliDefault"]);
});

it("changes who answers with one of three choices, and dims them while Hide AI is off, when a risk path is the person's again (B33, B10, B37)", async () => {
  const summary = { my_turn: 0, factories: [factory()], inbox: [] };
  const { container, events } = await mount(summary, { tab: "settings", factory: "f1" });
  await answerConfig(summary, events);
  await act(async () => container.querySelector<HTMLButtonElement>("[data-factory-mode='autonomous'] button")!.click());
  const write = lastAction(events);
  expect(write.payload.command).toEqual({ verb: "config", project: "/fixture", set: [["observer_mode", "autonomous"]] });
  await act(async () => useShellStore.setState({ factory: { summary, actions: [{ request_id: write.payload.request_id, answer: { ok: true, config: { ...CONFIG, observer_mode: "autonomous" }, machine: { max_workers: 5 } } }] } }));
  expect(container.querySelector("[data-factory-risk-note]")!.textContent).toBe(english["factory.settings.riskAi"]);
  await act(async () => useShellStore.setState((state) => ({ rest: { ...state.rest, status: { ...state.rest?.status, background_ai: { enabled: false, provider: null, chosen: false, providers: [], unavailable_reason: null } } } as never })));
  expect(container.querySelector("[data-factory-ai-off]")!.textContent).toBe(english["factory.settings.aiOff"]);
  expect(container.querySelector("[data-factory-risk-note]")!.textContent).toBe(english["factory.settings.riskMine"]);
});

it("lets a person answer a decision Factory AI made differently from the Task page while the engine allows it (D-19, B29)", async () => {
  const at = NOW - 60_000;
  const answered: TaskDetail = {
    ...detail("running", ["pause", "cancel"]),
    decisions: [decision({ id: "R1", text: "정렬은 web 쪽에서", by: "worker", at: at - 1 }), decision({ id: "R2", text: "정렬 키? -> updated_at", by: "ai", source: "answer", kind: "B", at, overridable: true })],
  };
  const { container, events } = await mount({ my_turn: 0, factories: [factory()], inbox: [] }, { task: { factory: "f1", task: "T-1" } });
  await act(async () => useShellStore.setState({ factoryTask: { factory: "f1", task: "T-1", detail: answered } }));
  expect([...container.querySelectorAll("[data-factory-decision-by]")].map((by) => by.getAttribute("data-factory-decision-by"))).toEqual(["ai", "worker"]);
  await act(async () => container.querySelector<HTMLButtonElement>("[data-factory-decision-override='R2']")!.click());
  await act(async () => type(container.querySelector<HTMLInputElement>("[data-factory-override-text]")!, "last_activity"));
  await act(async () => container.querySelector("[data-factory-override-text]")!.closest("form")!.requestSubmit());
  expect(commands(events).at(-1)).toEqual({ verb: "answer", task: "f1/T-1", question: null, choice: null, text: "last_activity", decision: "R2" });
  const finished = { ...answered, card: card("T-1", "done"), decisions: answered.decisions.map((row) => ({ ...row, overridable: false })) };
  await act(async () => useShellStore.setState({ factoryTask: { factory: "f1", task: "T-1", detail: finished } }));
  expect(container.querySelector("[data-factory-decision-override]")).toBeNull();
});

it("says why a worker stopped with Factory AI's reading, and how the engine treated its rest (B23, D-22)", async () => {
  const stopped: TaskDetail = { ...detail("stopped", ["retry", "cancel"]), stop_code: "no_report", diagnosis: "테스트 실행을 기다리다 멈춤", woke_at: NOW - 240_000, diagnosed_at: NOW - 120_000, diagnosed_from: "screen" };
  const { container } = await mount({ my_turn: 0, factories: [factory()], inbox: [] }, { task: { factory: "f1", task: "T-1" } });
  await act(async () => useShellStore.setState({ factoryTask: { factory: "f1", task: "T-1", detail: stopped } }));
  expect(container.querySelector("[data-factory-stop='no_report']")!.textContent).toBe(`${english["factory.stop.no_report"]} · ${english["factory.card.noReply"]}`);
  expect(container.querySelector("[data-factory-diagnosis]")!.textContent).toBe(`${english["factory.task.diagnosis"].replace("{{text}}", "테스트 실행을 기다리다 멈춤")} · ${english["factory.diagnosisSource.screen"]}`);
  expect(container.querySelector("[data-factory-rest]")!.textContent).toBe(`${english["factory.task.woke"]}${english["factory.task.noReply"]}${english["factory.task.diagnosed"]}`);
});

it("marks Factory AI's pick among the worker candidates before a worker starts (D-41)", async () => {
  const workers = [{ agent: "codex", model: "gpt-6.1-sol", effort: "high", description: "대부분의 Task" }, { agent: "claude", model: "opus", effort: "max", description: "큰 리팩터" }];
  const waiting: TaskDetail = { ...detail("waiting", ["priority", "cancel"]), ai_picked_worker: 2, ai_pick_reason: "큰 변경" };
  const { container } = await mount({ my_turn: 0, factories: [factory({ workers })], inbox: [] }, { task: { factory: "f1", task: "T-1" } });
  await act(async () => useShellStore.setState({ factoryTask: { factory: "f1", task: "T-1", detail: waiting } }));
  expect(container.querySelector("[data-factory-worker-pick]")!.getAttribute("data-factory-worker-pick")).toBe("2");
  expect(container.querySelector("[data-factory-picked]")!.textContent).toBe(english["factory.task.picked"].replace("{{description}}", "큰 리팩터 · 큰 변경"));
});

/** The one-time line (PRD first-run-onboarding B43): `seen` is what the core says about it; null is a snapshot not yet read. */
function introduced(seen: boolean | null) {
  useShellStore.setState({ rest: seen === null ? null : ({ ui_state: { screen_intros_seen: seen ? ["factory"] : [] } } as unknown as SnapshotRest) });
}
const intros = (events: Parameters<DispatchFn>[0][]) => events.filter((event) => event.kind === "screen_intro_seen");
const LINE = english["firstRun.intro.factory"];

it("shows the one-time line under the title on the first visit, records it once drawn, and keeps it for the visit (B43)", async () => {
  introduced(false);
  const { container, events } = await mount({ my_turn: 0, factories: [], inbox: [] });
  const line = container.querySelector('[data-screen-intro="factory"]');
  expect(line?.textContent).toBe(LINE);
  // Under the title, above the way to make a Factory.
  expect(line?.previousElementSibling?.querySelector("h1")?.textContent).toBe(english["factory.title"]);
  expect(intros(events)).toEqual([{ schema_version: 2, kind: "screen_intro_seen", payload: { screen: "factory" } }]);
  await act(async () => introduced(true));
  expect(container.querySelector('[data-screen-intro="factory"]')).not.toBeNull();
});

it("does not show the line once it was seen, and records nothing (B43)", async () => {
  introduced(true);
  const { container, events } = await mount({ my_turn: 0, factories: [factory()], inbox: [] });
  expect(container.querySelector("[data-screen-intro]")).toBeNull();
  expect(intros(events)).toEqual([]);
});

it("records the line only when it draws: not while the summary loads or on a Task page opened first (B43)", async () => {
  introduced(false);
  const loading = await mount(null as unknown as FactorySummary);
  expect(loading.container.querySelector("[data-screen-intro]")).toBeNull();
  expect(intros(loading.events)).toEqual([]);
  await act(async () => useShellStore.setState({ factory: { summary: { my_turn: 0, factories: [factory()], inbox: [] }, actions: [] } }));
  expect(loading.container.querySelector('[data-screen-intro="factory"]')?.textContent).toBe(LINE);
  expect(intros(loading.events)).toHaveLength(1);
  await act(async () => root?.unmount());

  introduced(false);
  const task = await mount({ my_turn: 0, factories: [factory()], inbox: [] }, { task: { factory: "f1", task: "T-1" } });
  expect(task.container.querySelector("[data-screen-intro]")).toBeNull();
  expect(intros(task.events)).toEqual([]);
});
