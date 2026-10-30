// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, afterEach, expect, it, vi } from "vitest";
import { emptyScope } from "../test/legacyAgentScope";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { createActions } from "./actions";
import type { AgentScope, TaskCardScope, TaskState } from "./agentScope";
import { TooltipProvider } from "./components/ui/tooltip";
import { clientI18n } from "./i18n/translator";
import type { AgentRow, Checkout, SessionPull, SnapshotRest, TaskView, Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { TaskLens } from "./TaskLens";
import { useUiStore } from "./ui";

// The sidebar's 태스크 list over a device scope the core already decided
// (PRD sidebar-task-lens B4-B18). Expected groups, order and lines come from
// the PRD, not from the list's own code.

const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());

type Sent = { kind: string; payload: Record<string, unknown> };

function row(id: string, over: Partial<AgentRow> = {}, session: Partial<AgentRow["state"]["session"]> = {}): AgentRow {
  const base = legacyAgentRow({ id, pane_id: id, identity_label: `작업 ${id}`, agent_kind: "claude", mark: "going", group: "working", demand: "none", unread: false, status_code: "working", emphasized: false, changed_at_unix_ms: 1, session_id: `s-${id}`, ...over });
  return { ...base, state: { ...base.state, session: { group: "working", line: null, unfinished: false, ...session } } } as AgentRow;
}

function card(root: string, state: TaskState, checkout: string, members: string[] = [root]): TaskCardScope {
  return { root, state, members, checkouts: [checkout], recency: "" };
}

/** Two projects on this Mac, each with one checkout, and the device's tasks in the core's order. */
function snapshot(agents: AgentRow[], cards: TaskCardScope[], view: TaskView = "status", folds: string[] = []): SnapshotRest {
  const scope: AgentScope = { ...emptyScope(), tasks: { cards, needs_you: cards.filter((entry) => entry.state === "needs_you").length, recency: "" } };
  const checkout = (id: string, project: string) => ({ id, workspace_id: project, label: "main", branch: "main", path: `/${project}`, agent_scope: emptyScope(), tabs: [{ id: `${id}-tab`, panes: agents.map((agent) => ({ id: agent.pane_id })) }] }) as unknown as Checkout;
  const project = (id: string, label: string) => ({ id, device_id: "local", label, path: `/${id}`, is_git: true, agent_scope: emptyScope(), checkouts: [checkout(`${id}-main`, id)] }) as unknown as Workspace;
  return {
    navigator: { devices: [{ id: "local", kind: "local", label: "Local", agent_scope: scope }], focused_device_id: "local", focused_checkout_id: "hide-main", workspaces: [project("hide", "herdr-ide"), project("site", "site")], agents },
    ui_state: { task_view: view, sessions_expanded_agent_pane_ids: folds, left_sidebar_visible: true },
  } as unknown as SnapshotRest;
}

let cleanup: (() => Promise<void>) | null = null;
afterEach(async () => {
  await cleanup?.();
  cleanup = null;
});

async function render(rest: SnapshotRest, agents: AgentRow[], focused: string | null = null) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  await clientI18n.changeLanguage("ko");
  const shell = useShellStore.getState();
  const ui = useUiStore.getState();
  useShellStore.setState({ rest, agents, connection: "live", focusedPaneId: focused });
  useUiStore.setState({ screen: { kind: "workspace" }, overviewOpen: false });
  const sent: Sent[] = [];
  const actions = createActions((event) => { sent.push(event as Sent); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><TaskLens actions={actions} /></TooltipProvider>));
  cleanup = async () => {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    await clientI18n.changeLanguage("en");
    vi.unstubAllGlobals();
  };
  return { container, sent };
}

const heads = (container: HTMLElement) => [...container.querySelectorAll("[data-task-group-head]")].map((head) => head.textContent);
const rows = (container: HTMLElement) => [...container.querySelectorAll("[data-task-row]")].map((element) => element.getAttribute("data-task-row"));

it.each(["status", "project"] as const)("in %s orders resting tasks by their displayed request age, with unknown times last", async (view) => {
  const agents = [row("old", { last_prompt_at_unix_ms: 100, changed_at_unix_ms: 900 }), row("unknown", { last_prompt_at_unix_ms: null }), row("recent", { last_prompt_at_unix_ms: 300, changed_at_unix_ms: 1 }), row("middle", { last_prompt_at_unix_ms: 200 })];
  const cards = agents.map((agent) => card(agent.pane_id, "resting", "hide-main"));
  const { container } = await render(snapshot(agents, cards, view), agents);
  await act(async () => container.querySelector<HTMLButtonElement>("[data-task-fold] > button")!.click());
  expect(rows(container)).toEqual(["recent", "middle", "old", "unknown"]);
  const refreshed = agents.map((agent) => ({ ...agent, changed_at_unix_ms: agent.pane_id === "old" ? 10_000 : 1 }));
  await act(async () => useShellStore.setState({ agents: refreshed, rest: snapshot(refreshed, cards, view) }));
  expect(rows(container)).toEqual(["recent", "middle", "old", "unknown"]);
});

it("groups the device's tasks as 내 차례 (a block included), 읽을 것, 작업 중 and 대기, drops empty groups, names each row's project, and folds the resting and cleanup ones (B4, B5, B9; status-marks B32)", async () => {
  const agents = [
    row("ask", { status_code: "question", state: undefined as never }, { group: "needs_you" }),
    row("stuck", { status_code: "error", demand: "error", mark: "turn" }, { group: "needs_you", line: "API 서버에 닿지 못해 멈춤" }),
    row("waits"), row("works", {}, { line: "테스트 돌리는 중" }), row("done"), row("rest"), row("clean"),
  ];
  agents[0] = { ...agents[0]!, state: { ...agents[0]!.state, ask: { verb: "answer", what: "새 언어로 다시 쓸까요?", more: 0 } } };
  const cards = [card("ask", "needs_you", "hide-main"), card("stuck", "blocked", "hide-main"), card("waits", "waiting", "site-main"), card("works", "working", "hide-main"), card("done", "done", "site-main"), card("rest", "resting", "hide-main"), card("clean", "cleanup", "hide-main")];
  const { container } = await render(snapshot(agents, cards), agents);
  expect(heads(container)).toEqual(["내 차례2", "읽을 것1", "작업 중1", "대기1"]);
  expect(rows(container)).toEqual(["ask", "stuck", "done", "works", "waits"]);
  expect(container.querySelector("[data-task-count]")?.textContent).toBe("태스크 5");
  const line = (pane: string) => container.querySelector(`[data-task-row="${pane}"] [data-task-line]`);
  // A question leads with its verb, as the sidebar row does (status-marks B5, B33).
  expect(line("ask")?.textContent).toBe("herdr-ide질문 · 새 언어로 다시 쓸까요?");
  expect(line("ask")?.querySelector(".text-warning")).not.toBeNull();
  // A block is the operator's turn: its cause in the warning colour, led by 막힘.
  expect(line("stuck")?.querySelector(".text-warning")?.textContent).toBe("막힘 · API 서버에 닿지 못해 멈춤");
  expect(line("waits")?.textContent).toBe("site");
  // The folds open in place and close with 접기.
  const folds = [...container.querySelectorAll<HTMLElement>("[data-task-fold] > button")];
  expect(folds.map((fold) => fold.textContent)).toEqual(["쉬는 중 1", "정리할 것 1"]);
  await act(async () => folds[0]!.click());
  expect(rows(container)).toContain("rest");
  await act(async () => container.querySelector<HTMLElement>('[data-task-focus="fold-close"]')!.click());
  expect(rows(container)).not.toContain("rest");
});

it("in 상태별 holds 내 차례 and 읽을 것 still above the rest, arrows crossing both, and not when nothing is under them", async () => {
  const agents = [row("ask", {}, { group: "needs_you" }), row("done"), row("works"), row("rest")];
  const cards = [card("ask", "needs_you", "hide-main"), card("done", "done", "hide-main"), card("works", "working", "hide-main"), card("rest", "resting", "hide-main")];
  const groupsIn = (container: HTMLElement, part: string) => [...container.querySelectorAll(`[data-task-scroll="${part}"] [data-task-group]`)].map((group) => group.getAttribute("data-task-group"));
  const status = await render(snapshot(agents, cards), agents);
  expect(groupsIn(status.container, "raised")).toEqual(["needs_you", "done"]);
  expect(groupsIn(status.container, "list")).toEqual(["working"]);
  const doneRow = status.container.querySelector<HTMLElement>('[data-task-row="done"] [data-task-focus="row"]')!;
  doneRow.focus();
  await act(async () => doneRow.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })));
  expect(document.activeElement?.closest('[data-task-group="working"]')).not.toBeNull();
  await cleanup?.();
  // With only those groups and no fold under them, they are the whole list.
  const alone = await render(snapshot(agents.slice(0, 2), cards.slice(0, 2)), agents.slice(0, 2));
  expect(alone.container.querySelector('[data-task-scroll="raised"]')).toBeNull();
  expect(groupsIn(alone.container, "list")).toEqual(["needs_you", "done"]);
  await cleanup?.();
  const project = await render(snapshot(agents, cards, "project"), agents);
  expect(project.container.querySelector('[data-task-scroll="raised"]')).toBeNull();
});

it("narrows to the project the chip chose, leaves the project's name off its rows, and keeps the other projects' 내 차례 as +N that lifts the choice", async () => {
  const agents = [row("ask", {}, { group: "needs_you" }), row("works"), row("elsewhere", {}, { group: "needs_you" })];
  const cards = [card("ask", "needs_you", "hide-main"), card("works", "working", "hide-main"), card("elsewhere", "needs_you", "site-main")];
  const rest = snapshot(agents, cards);
  const all = await render(rest, agents);
  expect(all.container.querySelector("[data-sidebar-project-trigger]")?.textContent).toBe("모든 프로젝트");
  expect(all.container.querySelector("[data-sidebar-project-elsewhere]")).toBeNull();
  await cleanup?.();
  cleanup = null;
  const hide = rest.navigator!.workspaces![0]!;
  const hideCards = cards.slice(0, 2);
  hide.agent_scope = { ...emptyScope(), tasks: { cards: hideCards, needs_you: 1, recency: "" } };
  rest.ui_state = { ...rest.ui_state!, sidebar_project: { device_id: "local", workspace_id: "hide" } };
  const { container, sent } = await render(rest, agents);
  expect(rows(container)).toEqual(["ask", "works"]);
  expect(container.querySelector("[data-task-project]")).toBeNull();
  expect(container.querySelector("[data-sidebar-project-trigger]")?.textContent).toBe("herdr-ide");
  const badge = container.querySelector<HTMLButtonElement>("[data-sidebar-project-elsewhere]");
  expect(badge?.textContent).toBe("+1");
  await act(async () => badge!.click());
  expect(sent.filter((event) => event.kind === "sidebar_project_set").map((event) => event.payload)).toEqual([{ project: null }]);
});

it("in 프로젝트별 draws one group per project, the asking project first, with no project name on the rows and a state label over each band (B6; status-marks B32)", async () => {
  const agents = [row("a"), row("b", {}, { group: "needs_you" }), row("c"), row("d")];
  const cards = [card("a", "working", "hide-main"), card("b", "needs_you", "site-main"), card("c", "done", "hide-main"), card("d", "waiting", "hide-main")];
  const { container } = await render(snapshot(agents, cards, "project"), agents);
  expect([...container.querySelectorAll("[data-task-group]")].map((group) => group.getAttribute("data-task-group"))).toEqual(["site", "hide"]);
  expect(container.querySelector("[data-task-project]")).toBeNull();
  expect(rows(container)).toEqual(["b", "c", "a", "d"]);
  const bands = (project: string) => [...container.querySelectorAll(`[data-task-group="${project}"] [data-task-band]`)].map((band) => band.textContent);
  expect(bands("site")).toEqual(["내 차례"]);
  expect(bands("hide")).toEqual(["읽을 것", "작업 중", "대기"]);
  const label = container.querySelector('[data-task-group="site"] [data-task-band]')!;
  expect(label.getAttribute("aria-hidden")).toBe("true");
  expect(label.className).toContain("text-warning");
  expect([...container.querySelectorAll('[data-task-group="hide"] ul[aria-label]')].map((list) => list.getAttribute("aria-label"))).toEqual(["읽을 것", "작업 중", "대기"]);
  expect(container.querySelector('[data-task-group-head="project"]')?.textContent).toBe("site내 차례 1");
  expect([...container.querySelectorAll('[data-task-group-head="project"]')][1]?.textContent).toBe("herdr-ide읽을 것 1작업 중 1대기 1");
});

it("in 프로젝트별 puts a project whose only turn is a block first, as a block is the operator's turn (status-marks B32)", async () => {
  const agents = [row("a"), row("b", { status_code: "error", demand: "error", mark: "turn" }, { group: "needs_you" })];
  const { container } = await render(snapshot(agents, [card("a", "working", "hide-main"), card("b", "blocked", "site-main")], "project"), agents);
  expect([...container.querySelectorAll("[data-task-group]")].map((group) => group.getAttribute("data-task-group"))).toEqual(["site", "hide"]);
});

it("opens a task's pane on a press, its record on Space, and resolves it from the ⋯ menu (B12, B15, B18)", async () => {
  const agents = [row("a", {}, { line: "일하는 중" })];
  const rest = snapshot(agents, [card("a", "working", "hide-main")]);
  (rest as { workspace_view?: unknown }).workspace_view = { device_id: "local", path: "/hide", open: false, documents: false, tool: null, pinned: false, document_width: null, detail_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0, lane_called_side: "view" };
  const { container, sent } = await render(rest, agents);
  const button = container.querySelector<HTMLElement>('[data-task-row="a"] [data-task-focus="row"]')!;
  await act(async () => button.click());
  expect(sent.at(-1)?.kind).toBe("focus_pane");
  sent.length = 0;
  await act(async () => button.dispatchEvent(new KeyboardEvent("keydown", { key: " ", bubbles: true })));
  expect(sent.map((event) => event.kind)).toEqual(["detail_open"]);
  expect(sent[0]!.payload).toEqual({ item: { kind: "record", session: { device_id: "local", agent: "claude", session_id: "s-a" } } });
});

it("selects the row of the pane with the keyboard, and its nearest drawn ancestor while it is folded away (B13)", async () => {
  const agents = [row("parent", { lineage_child_pane_ids: ["child"] }), row("child", { lineage_parent_pane_id: "parent", delegated: true }), row("other")];
  const cards = [card("parent", "working", "hide-main", ["parent", "child"]), card("other", "working", "hide-main")];
  const { container } = await render(snapshot(agents, cards), agents, "child");
  expect(container.querySelector('[data-task-child="child"]')).toBeNull();
  expect(container.querySelector('[data-task-row="parent"]')?.getAttribute("data-selected")).toBe("true");
  expect(container.querySelector('[data-task-row="other"]')?.getAttribute("data-selected")).toBeNull();
  // Folded, the parent counts no children; only a raise below it would draw `! N` (status-marks B35).
  expect(container.querySelector('[data-task-row="parent"] [data-task-children]')).toBeNull();
  expect(container.querySelector('[data-task-row="parent"] [data-descendant-mark]')).toBeNull();
});

it.each(["status", "project"] as const)("in %s reveals a resting pane on entry, keeps a manual fold closed until re-entry, and leaves other folds closed", async (view) => {
  const agents = [row("rest"), row("other"), row("clean"), row("works")];
  const cards = [card("rest", "resting", "hide-main"), card("other", "resting", "site-main"), card("clean", "cleanup", "hide-main"), card("works", "working", "hide-main")];
  const rest = snapshot(agents, cards, view);
  const { container } = await render(rest, agents, "rest");
  const selected = () => container.querySelector('[data-task-row="rest"][data-selected="true"]');
  expect(selected()).not.toBeNull();
  expect(container.querySelector('[data-task-row="clean"]')).toBeNull();
  if (view === "project") expect(container.querySelector('[data-task-row="other"]')).toBeNull();
  await act(async () => container.querySelector<HTMLElement>('[data-task-fold="rest"] > button')!.click());
  expect(selected()).toBeNull();
  // An unrelated snapshot must not undo the operator's explicit collapse.
  await act(async () => useShellStore.setState({ rest: snapshot(agents, cards, view) }));
  expect(selected()).toBeNull();
  await act(async () => useShellStore.setState({ focusedPaneId: "works" }));
  await act(async () => useShellStore.setState({ focusedPaneId: "rest" }));
  expect(selected()).not.toBeNull();
});

it.each(["status", "project"] as const)("in %s reveals the resting task for a focused child and when its task moves to rest", async (view) => {
  const agents = [row("parent", { lineage_child_pane_ids: ["child"] }), row("child", { lineage_parent_pane_id: "parent", delegated: true })];
  const { container } = await render(snapshot(agents, [card("parent", "done", "hide-main", ["parent", "child"])], view), agents, "child");
  await act(async () => useShellStore.setState({ rest: snapshot(agents, [card("parent", "resting", "hide-main", ["parent", "child"])], view) }));
  expect(container.querySelector('[data-task-fold="rest"] > button')?.getAttribute("aria-expanded")).toBe("true");
  expect(container.querySelector('[data-task-row="parent"]')?.getAttribute("data-selected")).toBe("true");
});

it("reveals the focused resting task when changing the grouping or returning from Overview", async () => {
  const agents = [row("rest")];
  const cards = [card("rest", "resting", "hide-main")];
  const { container } = await render(snapshot(agents, cards), agents, "rest");
  await act(async () => useShellStore.setState({ rest: snapshot(agents, cards, "project") }));
  expect(container.querySelector('[data-task-row="rest"][data-selected="true"]')).not.toBeNull();
  await act(async () => container.querySelector<HTMLElement>('[data-task-fold="rest"] > button')!.click());
  await act(async () => useUiStore.setState({ overviewOpen: true }));
  expect(container.querySelector('[data-task-row="rest"]')).toBeNull();
  await act(async () => useUiStore.setState({ overviewOpen: false }));
  expect(container.querySelector('[data-task-row="rest"][data-selected="true"]')).not.toBeNull();
});

it("numbers the task rows top to bottom while ⌥ is held, an opened fold's rows after the groups, and a task's children never (electron-digit-shortcuts-hints D-02)", async () => {
  const agents = [row("ask", {}, { group: "needs_you" }), row("parent", { lineage_child_pane_ids: ["child"] }), row("child", { lineage_parent_pane_id: "parent", delegated: true }), row("done"), row("rest")];
  const cards = [card("ask", "needs_you", "hide-main"), card("parent", "working", "hide-main", ["parent", "child"]), card("done", "done", "site-main"), card("rest", "resting", "hide-main")];
  const { container } = await render(snapshot(agents, cards, "status", ["parent"]), agents);
  const caps = () => [...container.querySelectorAll("[data-task-row], [data-task-child]")].map((item) => `${item.getAttribute("data-task-row") ?? item.getAttribute("data-task-child")}:${item.querySelector("[data-keycap]")?.textContent ?? "-"}`);
  // Nothing before the hold reveals.
  expect(container.querySelector('[data-task-child="child"]')).not.toBeNull();
  expect(container.querySelector("[data-keycap]")).toBeNull();
  await act(async () => useUiStore.getState().setHint("agents"));
  expect(caps()).toEqual(["ask:1", "done:2", "parent:3", "child:-"]);
  // The fold opened on this page draws its rows, and they take the next numbers.
  await act(async () => container.querySelector<HTMLElement>("[data-task-fold] > button")!.click());
  expect(caps()).toEqual(["ask:1", "done:2", "parent:3", "child:-", "rest:4"]);
  await act(async () => useUiStore.getState().setHint(null));
  expect(container.querySelector("[data-keycap]")).toBeNull();
});

it("draws the first step when the device has no project, and New agent when it has no task (B20, B42)", async () => {
  const empty = { ...snapshot([], []), navigator: { ...snapshot([], []).navigator, workspaces: [] } } as SnapshotRest;
  const none = await render(empty, []);
  expect(none.container.querySelector("[data-tasks-no-project]")?.textContent).toContain("프로젝트를 추가하면 에이전트를 시작할 수 있어요.");
  await cleanup?.();
  cleanup = null;
  const idle = await render(snapshot([], []), []);
  expect(idle.container.querySelector("[data-tasks-empty]")?.textContent).toContain("진행 중인 태스크 없음");
});

// A task row draws the row's own mark and line, as the sidebar's (status-marks D-12, D-22, D-24, B16, B33, B35).
it("draws each task's row mark and line as the sidebar does, counts no folded children or raises, and dims only an unknown row", async () => {
  const waits = row("waits", { mark: "waiting", status_code: "waiting", wait: "checks", activity: "stopped" });
  waits.state = { ...waits.state, line: { text: "", mode: "waiting", tone: { kind: "subtle", read: false }, pull_request: 58 } };
  const agents = [
    waits,
    row("left", { mark: "unread", status_code: "unfinished", activity: "stopped", unread: true }, { group: "done", line: "테스트 환경 연결 남음", unfinished: true }),
    row("read", { mark: "quiet", status_code: "unfinished", activity: "stopped" }, { group: "idle", line: "문서 정리 남음", unfinished: true }),
    row("sleep", { mark: "asleep", status_code: "sleeping", activity: "stopped" }, { line: "파서를 나눴어요" }),
    row("lost", { mark: "unknown", status_code: "unknown", activity: "unknown" }),
    row("parent", { lineage_child_pane_ids: ["k1", "k2"], descendant_mark: { kind: "raised", count: 1 } }),
    row("k1", { lineage_parent_pane_id: "parent", delegated: true, mark: "quiet", status_code: "question", demand: "question" }),
    row("k2", { lineage_parent_pane_id: "parent", delegated: true, mark: "done", status_code: "done", activity: "stopped" }),
  ];
  const cards = [card("waits", "waiting", "hide-main"), card("left", "done", "hide-main"), card("read", "resting", "hide-main"), card("sleep", "working", "hide-main"), card("lost", "working", "hide-main"), card("parent", "working", "hide-main", ["parent", "k1", "k2"])];
  const { container } = await render(snapshot(agents, cards), agents);
  await act(async () => container.querySelector<HTMLElement>('[data-task-fold="rest"] > button')!.click());
  const at = (pane: string) => container.querySelector(`[data-task-row="${pane}"]`)!;
  const mark = (pane: string) => at(pane).querySelector("[data-mark]")?.getAttribute("data-mark");
  const said = (pane: string) => [...at(pane).querySelectorAll("[data-task-line] [data-line-part]")].map((part) => part.textContent).join("");
  expect(mark("waits")).toBe("waiting");
  expect(said("waits")).toBe("CI 기다림 · PR #58");
  // The row's name and hover say the mark as the sidebar does (B3, B33).
  const named = (pane: string) => at(pane).querySelector('[data-task-focus="row"]')!;
  expect(named("waits").getAttribute("aria-label")).toContain("진행 중 · CI 기다림");
  expect(named("waits").getAttribute("title")).toContain("CI 기다림 · PR #58");
  expect(named("waits").getAttribute("title")).toContain("진행 중 · CI 기다림");
  expect(named("left").getAttribute("aria-label")).toContain("읽을 것 · 일 남음");
  expect(said("left")).toBe("일 남음 · 테스트 환경 연결 남음");
  expect(mark("left")).toBe("unread");
  // Read, work left draws no mark and a quiet line; no warning ◐ anywhere.
  expect(mark("read")).toBe("quiet");
  expect(said("read")).toBe("일 남음 · 문서 정리 남음");
  expect(container.textContent).not.toContain("◐");
  expect(mark("sleep")).toBe("asleep");
  expect(said("sleep")).toBe("");
  expect(said("lost")).toBe("상태 모름");
  expect(at("lost").className).toContain("opacity-(--opacity-dimmed)");
  for (const pane of ["waits", "left", "read", "sleep", "parent"]) expect(at(pane).className).not.toContain("opacity-(--opacity-dimmed)");
  // A folded parent counts no children, and a raise below it draws no `!N`:
  // its root stands in 내 차례 with the raised ask on its line (needs-you-cleanup B7).
  expect(at("parent").querySelector("[data-task-children]")).toBeNull();
  expect(at("parent").querySelector("[data-descendant-mark]")).toBeNull();
});

it("keeps a read question in 내 차례 dimmed with a hollow ! and 읽음, and its ☆ puts it off to 다시 볼 것 (needs-you-cleanup B10, B13)", async () => {
  const agents = [
    row("asked", { mark: "quiet", group: "seen", demand: "question", activity: "idle", revisit: { starred: false, read_ask: true, since_unix_ms: 1 } } as Partial<AgentRow>),
    row("kept", { mark: "quiet", group: "seen", activity: "idle", revisit: { starred: true, read_ask: false, since_unix_ms: 1 } } as Partial<AgentRow>),
  ];
  const { container, sent } = await render(snapshot(agents, [{ ...card("asked", "needs_you", "hide-main"), read: true }, card("kept", "revisit", "hide-main")]), agents);
  const at = (pane: string) => container.querySelector(`[data-task-row="${pane}"]`)!;
  expect(at("asked").getAttribute("data-task-read")).toBe("true");
  expect(at("asked").querySelector("[data-mark]")?.getAttribute("data-mark-read")).toBe("true");
  expect(at("asked").querySelector("[data-task-read-tag]")?.textContent).toBe("읽음");
  expect(at("asked").querySelector('[data-task-focus="row"]')?.getAttribute("aria-label")).toContain("내 차례 · 읽은 질문");
  expect(at("kept").getAttribute("data-task-read")).toBeNull();
  const button = (id: string) => container.querySelector<HTMLButtonElement>(`[data-task-revisit="${id}"]`)!;
  expect(button("asked").getAttribute("aria-pressed")).toBe("false");
  expect(button("asked").getAttribute("aria-label")).toBe("다시 볼 것으로 표시");
  expect(button("kept").getAttribute("aria-pressed")).toBe("true");
  await act(async () => button("asked").click());
  await act(async () => button("kept").click());
  expect(sent.map((event) => [event.kind, event.payload.pane_id])).toEqual([["revisit_star", "asked"], ["revisit_done", "kept"]]);
});

it("draws a middle child's whole pull request list on its child row, its grandchild's included, as its pane and record do", async () => {
  // A -> B -> C: B made #41 and C made #42. The core lays one list on each
  // session (docs/status-model.md, One list per session); B's is both.
  const pull = (number: number, state: SessionPull["state"], holder: string, own: boolean): SessionPull => ({
    url: `https://github.com/acme/app/pull/${number}`, number, title: `PR ${number}`, state, own, holders: [holder], stale: false, last_read_at_unix_ms: null,
  });
  const agents = [
    row("a", { lineage_child_pane_ids: ["b"], pulls: { pulls: [pull(41, "mergeable", "작업 b", false), pull(42, "merged", "작업 c", false)] } }),
    row("b", { lineage_parent_pane_id: "a", lineage_child_pane_ids: ["c"], delegated: true, pulls: { pulls: [pull(41, "mergeable", "작업 b", true), pull(42, "merged", "작업 c", false)] } }),
    row("c", { lineage_parent_pane_id: "b", delegated: true, pulls: { pulls: [pull(42, "merged", "작업 c", true)] } }),
  ];
  const { container } = await render(snapshot(agents, [card("a", "working", "hide-main", ["a", "b", "c"])], "status", ["a", "b"]), agents);
  const mark = (selector: string) => container.querySelector(`${selector} [data-agent-pr]`);
  const counts = (selector: string) => mark(selector)?.querySelector("[data-pr-counts]")?.getAttribute("data-pr-counts");
  expect(container.querySelector('[data-task-child="b"]')).not.toBeNull();
  expect(counts('[data-task-child="b"]')).toBe("mergeable:1 merged:1");
  expect(counts('[data-task-row="a"]')).toBe("mergeable:1 merged:1");
  expect(mark('[data-task-child="c"]')?.textContent).toBe("#42");
});
