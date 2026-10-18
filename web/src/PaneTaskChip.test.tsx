// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { initializeInterfaceI18n } from "./i18n/instance";
import { PaneTaskChip } from "./PaneTaskChip";
import type { AgentRow, BackgroundAi, ChildProgress, SnapshotRest, WorkOutput, WorkProgress, WorkTask } from "./snapshot";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";

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
const task = (id: string, name: string, done = false): WorkTask => ({ id, chapter_id: "chapter-1", name, at_unix_ms: 1_000, last_at_unix_ms: 2_000, end: done ? "done" : "waiting", done, unjudged: false });
const pr = (number: number, taskId: string, state = "open"): WorkOutput => ({ kind: "pr", repository: "acme/app", number, title: `PR ${number}`, url: `https://github.com/acme/app/pull/${number}`, state, stale: false, last_read_at_unix_ms: null, created_at_unix_ms: 1_500, task_id: taskId, source: SESSION, source_name: "root" });
const progress = (tasks: WorkTask[], extra: Partial<WorkProgress> = {}): WorkProgress => ({ session: SESSION, done: tasks.filter((row) => row.done).length, total: tasks.length, tasks, outputs: [], children: null, ...extra });
/** On: the feature switch alone decides; whether a provider can answer now does not (B15). */
const AI_ON: BackgroundAi = { enabled: true, chosen: true, agent_summary: true, provider: null, providers: [] } as unknown as BackgroundAi;

function agent(line: string | null = "README 태그 고치는 중"): AgentRow {
  return legacyAgentRow({ pane_id: "p1", agent_kind: "claude", session_id: "s-1", device_id: "mac", identity_label: "root", group: "working", activity: "working", status_code: "working", detail: line ?? undefined } as Partial<AgentRow>);
}

async function mount(rows: WorkProgress[] | undefined, options: { ai?: BackgroundAi; line?: string | null } = {}) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  initializeInterfaceI18n("en");
  useShellStore.setState({
    agents: [agent(options.line)],
    rest: { navigator: { devices: [{ id: "mac", kind: "local" }] }, status: { background_ai: options.ai ?? AI_ON } } as unknown as SnapshotRest,
    linkSummaries: { projects: {}, filling: false, progress: rows },
  });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><PaneTaskChip paneId="p1" actions={actions} /></TooltipProvider>));
  const chip = () => container.querySelector<HTMLButtonElement>("[data-pane-task-chip=p1]");
  const list = () => document.querySelector<HTMLElement>("[data-pane-task-list=p1]");
  return { container, actions, events, chip, list, unmount: async () => { await act(async () => root.unmount()); } };
}

it("shows done over tasks from one task, and nothing for a pane with no task", async () => {
  const one = await mount([progress([task("task-3", "README 번역 정리")])]);
  try {
    expect(one.chip()?.textContent).toBe("0/1");
    expect(one.chip()?.getAttribute("aria-label")).toBe("0 of 1 assigned tasks done");
  } finally { await one.unmount(); }
  // A ledger with no judgment publishes no progress for the session.
  const none = await mount(undefined);
  try {
    expect(none.chip()).toBeNull();
  } finally { await none.unmount(); }
});

it("never puts a child count on the chip; the list says the children in one line, the done count only once every child is judged (B15)", async () => {
  const children: ChildProgress = { children: ["done", "done", "working", "done", "done"], done: 4 };
  const view = await mount([progress([task("task-3", "가"), task("task-3.2", "나"), task("task-3.3", "다"), task("task-3.4", "라")], { children })]);
  try {
    expect(view.chip()?.textContent).toBe("0/4");
    await act(async () => view.chip()!.focus());
    expect(view.list()?.querySelector("[data-task-list-children]")?.textContent).toBe("Delegated5· Done 4");
    expect(view.list()!.querySelector("[data-child-segment], [data-child-progress]")).toBeNull();
  } finally { await view.unmount(); }
  const unjudged = await mount([progress([task("task-3", "가")], { children: { children: ["done", "unjudged"], done: null } })]);
  try {
    await act(async () => unjudged.chip()!.focus());
    expect(unjudged.list()!.querySelector("[data-task-list-children]")?.textContent).toBe("Delegated2");
  } finally { await unjudged.unmount(); }
});

it("puts this pane's own Now box right under the tasks, in the warning colour with `!` while it asks (B15)", async () => {
  const view = await mount([progress([task("task-3", "가")], { children: { children: ["working"], done: 0 } })]);
  try {
    await act(async () => view.chip()!.focus());
    const box = view.list()!.querySelector("[data-task-list-now]")!;
    expect([box.getAttribute("data-state-box"), box.textContent]).toEqual(["working", "Now · WorkingREADME 태그 고치는 중"]);
    // Tasks, then the box, then the children's line.
    expect(box.compareDocumentPosition(view.list()!.querySelector("[data-task-list-children]")!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  } finally { await view.unmount(); }
});

it("opens on the pointer and on focus, closes when both leave, and pins on a press until Esc", async () => {
  vi.useFakeTimers();
  const view = await mount([progress([task("task-3", "README의 HTML 태그가 보이게 하기"), task("task-3.2", "댓글 + 마크 위치 맞추기")])]);
  try {
    expect(view.list()).toBeNull();
    await act(async () => view.chip()!.dispatchEvent(new MouseEvent("pointerover", { bubbles: true })));
    expect(view.list()).not.toBeNull();
    expect([...view.list()!.querySelectorAll("[data-task-list-name]")].map((node) => node.textContent)).toEqual(["README의 HTML 태그가 보이게 하기", "댓글 + 마크 위치 맞추기"]);
    expect(view.list()!.querySelector("[data-task-list-line]")?.textContent).toBe("Now · WorkingREADME 태그 고치는 중");
    await act(async () => { view.chip()!.dispatchEvent(new MouseEvent("pointerout", { bubbles: true, relatedTarget: document.body })); vi.advanceTimersByTime(200); });
    expect(view.list()).toBeNull();
    await act(async () => view.chip()!.focus());
    expect(view.list()).not.toBeNull();
    await act(async () => view.chip()!.blur());
    expect(view.list()).toBeNull();
    await act(async () => view.chip()!.click());
    expect(useUiStore.getState().taskListPane).toBe("p1");
    await act(async () => { view.chip()!.dispatchEvent(new MouseEvent("pointerout", { bubbles: true, relatedTarget: document.body })); vi.advanceTimersByTime(200); });
    expect(view.list(), "a pinned list stays when the pointer leaves").not.toBeNull();
    const escape = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
    await act(async () => document.body.dispatchEvent(escape));
    expect(view.list()).toBeNull();
    expect(useUiStore.getState().taskListPane).toBeNull();
  } finally { await view.unmount(); }
});

it("pins the focused pane's list by the shortcut's action and closes it on a second press", async () => {
  const view = await mount([progress([task("task-3", "가")])]);
  try {
    useShellStore.setState({ focusedPaneId: "p1" });
    await act(async () => view.actions.toggleTaskList());
    expect(view.list()).not.toBeNull();
    await act(async () => view.actions.toggleTaskList());
    expect(view.list()).toBeNull();
  } finally { await view.unmount(); }
});

it("closes a pinned list on a second press of the chip, pointer down included", async () => {
  const view = await mount([progress([task("task-3", "가")])]);
  try {
    // A browser renders between the press and the click, as separate tasks.
    const press = async () => {
      await act(async () => {
        view.chip()!.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, cancelable: true, button: 0 }));
        view.chip()!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
        view.chip()!.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true, button: 0 }));
      });
      await act(async () => view.chip()!.click());
    };
    await press();
    expect(useUiStore.getState().taskListPane).toBe("p1");
    expect(view.list()).not.toBeNull();
    // The list listens for outside presses from the tick after it opens.
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
    await press();
    expect(useUiStore.getState().taskListPane, "the press that dismisses must not pin again").toBeNull();
    expect(view.list()).toBeNull();
  } finally { await view.unmount(); }
});

it("pins nothing by the shortcut on a pane with no chip", async () => {
  const view = await mount([progress([task("task-3", "가")])]);
  try {
    // A shell pane draws no chip, so nothing would ever clear a pin left on it.
    useShellStore.setState({ focusedPaneId: "shell-pane" });
    await act(async () => view.actions.toggleTaskList());
    expect(useUiStore.getState().taskListPane).toBeNull();
  } finally { await view.unmount(); }
});

it("opens the record from the list on the same session", async () => {
  const view = await mount([progress([task("task-3", "가")])]);
  try {
    const open = vi.spyOn(view.actions, "openSessionRecord").mockImplementation(() => {});
    await act(async () => view.chip()!.click());
    await act(async () => view.list()!.querySelector<HTMLButtonElement>("[data-task-list-record]")!.click());
    expect(open).toHaveBeenCalledWith(SESSION);
    expect(view.list()).toBeNull();
  } finally { await view.unmount(); }
});

it("draws a split request's pull request once, on its first task, and the shared state on every task", async () => {
  const tasks = [task("task-3", "가", false), task("task-3.2", "나", false)];
  const view = await mount([progress(tasks, { outputs: [pr(7, "task-3")] })]);
  try {
    await act(async () => view.chip()!.focus());
    const items = [...view.list()!.querySelectorAll("[data-task-list-item]")];
    expect(items[0]!.querySelector("[aria-label^='#7']")).not.toBeNull();
    expect(items[1]!.querySelector("[aria-label^='#7']")).toBeNull();
    expect(items.map((item) => item.querySelector("[data-task-state]")?.getAttribute("data-task-state"))).toEqual(["review", "review"]);
    // Nothing turns in the list (status-marks B34).
    expect(view.list()!.querySelector(".animate-spin")).toBeNull();
  } finally { await view.unmount(); }
});

it("keeps judged tasks without an error while no provider can answer, and draws no chip with Agent summaries off", async () => {
  const rows = [progress([task("task-3", "가", true)])];
  // No provider logged in, a failing provider, an older helper or an offline
  // device all leave the switch on: what was judged stays, with no error.
  const absent = await mount(rows, { ai: { ...AI_ON, providers: [] } as BackgroundAi });
  try {
    expect(absent.chip()?.textContent).toBe("1/1");
    expect(absent.container.querySelector("[role=alert]")).toBeNull();
  } finally { await absent.unmount(); }
  const off = await mount(rows, { ai: { ...AI_ON, agent_summary: false } as BackgroundAi });
  try {
    expect(off.chip()).toBeNull();
  } finally { await off.unmount(); }
});

it("draws a task left in progress on a stopped row as work left, with no ring", async () => {
  const view = await mount([progress([{ ...task("task-3", "가"), end: "waiting" }])]);
  try {
    useShellStore.setState({ agents: [{ ...agent(), mark: "quiet" }] });
    await act(async () => view.chip()!.focus());
    const item = view.list()!.querySelector("[data-task-list-item]")!;
    expect([item.querySelector("[data-task-state]")?.getAttribute("data-task-state"), item.querySelector("[data-mark]"), item.querySelector("[data-task-word]")?.textContent]).toEqual(["unfinished", null, "Unfinished"]);
  } finally { await view.unmount(); }
});

it("draws no current line when the agent has none rather than a placeholder", async () => {
  const view = await mount([progress([task("task-3", "가")])], { line: null });
  try {
    await act(async () => view.chip()!.focus());
    expect(view.list()!.querySelector("[data-task-list-line], [data-task-list-now]")).toBeNull();
  } finally { await view.unmount(); }
});

it("keeps `!` before a done task whose turn ended on a question and draws an unjudged task with no word (PRD task-continuation B3, B18)", async () => {
  const asked: WorkTask = { ...task("task-3", "쉬는 행 빈 원", true), end: "question" };
  const unknown: WorkTask = { ...task("task-5", "다른 일"), end: "unfinished", unjudged: true };
  const view = await mount([progress([asked, unknown], { children: { children: ["done", "done"], done: 2 } })]);
  try {
    expect(view.chip()?.textContent).toBe("1/2");
    await act(async () => view.chip()!.focus());
    const item = (id: string) => view.list()!.querySelector(`[data-task-list-item="${id}"]`)!;
    expect(item("task-3").querySelector("[data-task-box]")?.getAttribute("data-task-box")).toBe("checked");
    expect(item("task-3").querySelector("[data-task-asks]")).not.toBeNull();
    expect(item("task-5").querySelector("[data-task-box]")?.getAttribute("data-task-box")).toBe("empty");
    expect(item("task-5").querySelector("[data-task-word]")).toBeNull();
    expect(view.list()?.querySelector("[data-task-list-children]")?.textContent).toBe("Delegated2· Done 2");
  } finally { await view.unmount(); }
});
