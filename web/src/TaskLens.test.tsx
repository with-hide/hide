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
import type { AgentRow, Checkout, SnapshotRest, TaskView, Workspace } from "./snapshot";
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

it("groups the device's tasks as 내 차례 (a block included), 읽을 것, 일하는 중 and 대기, drops empty groups, names each row's project, and folds the resting and cleanup ones (B4, B5, B9; status-marks B32)", async () => {
  const agents = [
    row("ask", { status_code: "question", state: undefined as never }, { group: "needs_you" }),
    row("stuck", { status_code: "error", demand: "error", mark: "turn" }, { group: "needs_you", line: "API 서버에 닿지 못해 멈춤" }),
    row("waits"), row("works", {}, { line: "테스트 돌리는 중" }), row("done"), row("rest"), row("clean"),
  ];
  agents[0] = { ...agents[0]!, state: { ...agents[0]!.state, ask: { verb: "answer", what: "새 언어로 다시 쓸까요?", more: 0 } } };
  const cards = [card("ask", "needs_you", "hide-main"), card("stuck", "blocked", "hide-main"), card("waits", "waiting", "site-main"), card("works", "working", "hide-main"), card("done", "done", "site-main"), card("rest", "resting", "hide-main"), card("clean", "cleanup", "hide-main")];
  const { container } = await render(snapshot(agents, cards), agents);
  expect(heads(container)).toEqual(["내 차례2", "읽을 것1", "일하는 중1", "대기1"]);
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

it("in 프로젝트별 draws one group per project, the asking project first, with no project name on the rows and a state label over each band (B6; status-marks B32)", async () => {
  const agents = [row("a"), row("b", {}, { group: "needs_you" }), row("c"), row("d")];
  const cards = [card("a", "working", "hide-main"), card("b", "needs_you", "site-main"), card("c", "done", "hide-main"), card("d", "waiting", "hide-main")];
  const { container } = await render(snapshot(agents, cards, "project"), agents);
  expect([...container.querySelectorAll("[data-task-group]")].map((group) => group.getAttribute("data-task-group"))).toEqual(["site", "hide"]);
  expect(container.querySelector("[data-task-project]")).toBeNull();
  expect(rows(container)).toEqual(["b", "c", "a", "d"]);
  const bands = (project: string) => [...container.querySelectorAll(`[data-task-group="${project}"] [data-task-band]`)].map((band) => band.textContent);
  expect(bands("site")).toEqual(["내 차례"]);
  expect(bands("hide")).toEqual(["읽을 것", "일하는 중", "대기"]);
  const label = container.querySelector('[data-task-group="site"] [data-task-band]')!;
  expect(label.getAttribute("aria-hidden")).toBe("true");
  expect(label.className).toContain("text-warning");
  expect([...container.querySelectorAll('[data-task-group="hide"] ul[aria-label]')].map((list) => list.getAttribute("aria-label"))).toEqual(["읽을 것", "일하는 중", "대기"]);
  expect(container.querySelector('[data-task-group-head="project"]')?.textContent).toBe("site내 차례 1");
  expect([...container.querySelectorAll('[data-task-group-head="project"]')][1]?.textContent).toBe("herdr-ide읽을 것 1일하는 중 1대기 1");
});

it("in 프로젝트별 puts a project whose only turn is a block first, as a block is the operator's turn (status-marks B32)", async () => {
  const agents = [row("a"), row("b", { status_code: "error", demand: "error", mark: "turn" }, { group: "needs_you" })];
  const { container } = await render(snapshot(agents, [card("a", "working", "hide-main"), card("b", "blocked", "site-main")], "project"), agents);
  expect([...container.querySelectorAll("[data-task-group]")].map((group) => group.getAttribute("data-task-group"))).toEqual(["site", "hide"]);
});

it("opens a task's pane on a press, its record on Space, and resolves it from the ⋯ menu (B12, B15, B18)", async () => {
  const agents = [row("a", {}, { line: "일하는 중" })];
  const rest = snapshot(agents, [card("a", "working", "hide-main")]);
  (rest as { workspace_view?: unknown }).workspace_view = { device_id: "local", path: "/hide", tool: null, last_tool: null, prs: { only_checkout: false }, pinned: false, tree: false, lane_width: null, tree_width: null, lane_called: 0, lane_calls: 0 };
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
  expect(none.container.querySelector("[data-tasks-no-project]")?.textContent).toContain("Add a project to start an agent.");
  await cleanup?.();
  cleanup = null;
  const idle = await render(snapshot([], []), []);
  expect(idle.container.querySelector("[data-tasks-empty]")?.textContent).toContain("진행 중인 일감 없음");
});

// A task row draws the row's own mark and line, as the sidebar's (status-marks D-12, D-22, D-24, B16, B33, B35).
it("draws each task's row mark and line as the sidebar does, counts no folded children, and dims only an unknown row", async () => {
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
  expect(said("waits")).toBe("대기 · CI · PR #58");
  // The row's name and hover say the mark as the sidebar does (B3, B33).
  const named = (pane: string) => at(pane).querySelector('[data-task-focus="row"]')!;
  expect(named("waits").getAttribute("aria-label")).toContain("진행 중 · 대기");
  expect(named("waits").getAttribute("title")).toContain("대기 · CI · PR #58");
  expect(named("waits").getAttribute("title")).toContain("진행 중 · 대기");
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
  // A folded parent counts no children; a raise below it is its one mark.
  expect(at("parent").querySelector("[data-task-children]")).toBeNull();
  expect(at("parent").querySelector("[data-descendant-mark]")?.textContent).toBe("!1");
});
