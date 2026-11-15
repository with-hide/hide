// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, afterEach, expect, it, vi } from "vitest";
import tokensText from "../../design/tokens.json?raw";
import { emptyScope } from "../test/legacyAgentScope";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { createActions } from "./actions";
import type { AgentScope, AgentTreeScope, PrBoard, ToolCell } from "./agentScope";
import { TooltipProvider } from "./components/ui/tooltip";
import { clientI18n } from "./i18n/translator";
import { sidebarNumbers } from "./numbering";
import { Sidebar } from "./sidebar";
import type { AgentRow, Checkout, Device, SidebarLens, SnapshotRest, TaskView, Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

// Operator decision 2026-11-15: a tool's cell has the operator's 프로젝트 |
// 태스크 | PR switch and 상태 | 프로젝트 view switch, each list holding the
// tool's work alone, and nothing in it raised as the operator's turn. The
// trees, marks and boards here are what the core publishes; the test checks
// which of them each cell draws.

const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());

const TOOL = { connection: "c0ffee00c0ffee00", name: "hoyeon", owns: true };

function agent(id: string, over: Partial<AgentRow> = {}): AgentRow {
  const base = legacyAgentRow({ id, pane_id: id, identity_label: `작업 ${id}`, agent_kind: "claude", mark: "going", group: "working", demand: "none", unread: false, status_code: "working", emphasized: false, changed_at_unix_ms: 1, session_id: `s-${id}`, ...over });
  return { ...base, state: { ...base.state, session: { group: "working", line: null, unfinished: false } } } as AgentRow;
}

const AGENTS = [
  agent("mine", { group: "needs_you", mark: "turn", demand: "question", status_code: "question" }),
  agent("tool-root", { tool: TOOL, lineage_child_pane_ids: ["tool-child"] } as Partial<AgentRow>),
  agent("tool-child", { tool: TOOL, delegated: true, lineage_parent_pane_id: "tool-root" } as Partial<AgentRow>),
  agent("theirs"),
];

const tree = (rows: [string, number][]): AgentTreeScope => {
  const refs = rows.map(([pane_id, depth]) => ({ pane_id, occurrence: 0, depth }));
  return { rows: refs.filter((row) => row.depth === 0), visible_rows: refs, shown: [], more: 0, needs_you: false, turn_kind: null };
};

function checkout(id: string, project: string, panes: string[], sidebar: [string, number][], primary: boolean): Checkout {
  return {
    id, workspace_id: project, path: `/fixture/${id}`, label: id, branch: id, purpose: null, is_primary: primary, is_worktree: !primary, exists: true, has_panes: panes.length > 0,
    pull_request: null, active_tab_id: null, strip: [], next_tab_label: "1",
    tabs: panes.map((pane) => ({ id: `${id}:${pane}`, panes: [{ id: pane }] })),
    agent_scope: { ...emptyScope(), sidebar_tree: tree(sidebar) },
    agent_summary: { representative_pane_id: null, needs_you: 0, done: 0, working: 0, seen: 0, unknown: 0, marks: { turn: 0, going: 0, done: 0, unread: 0 } },
    github: { available: true, last_success_at_unix_ms: 1 },
  } as unknown as Checkout;
}

/** The operator named an empty tab of the checkout, which their tree draws as its own line. */
function withNamedTab(value: Checkout): Checkout {
  const named = { id: `${value.id}:notes`, panes: [], sidebar: { number: 3, manual_name: "notes", named_by_operator: true } };
  return { ...value, tabs: [...value.tabs, named] } as unknown as Checkout;
}

const prRow = (number: number, checkout: string): PrBoard["rows"][number] => ({ number, group: "working", fold: null, reason: { kind: "working" }, agents: [], agent_pane: null, checkout_id: checkout, checkout_ids: [checkout], issue: null });
const pull = (number: number, branch: string) => ({ number, title: `PR ${number}`, url: `https://github.com/fixture/alpha/pull/${number}`, badge: "open", review: null, is_draft: false, state: "pending", head_branch: branch });

function project(id: string, checkouts: Checkout[], prs: PrBoard["rows"] = [], pulls: ReturnType<typeof pull>[] = []): Workspace {
  return {
    id, label: id, path: `/fixture/${id}`, device_id: "local", registered: true, temporary: false, pinned: false, is_git: true, expanded: true,
    inactive_checkouts: { expanded: true, checkout_ids: [] },
    pull_requests: pulls,
    agent_scope: { ...emptyScope(), prs: { rows: prs, groups: prs.length ? [{ group: "working", numbers: prs.map((row) => row.number) }] : [], needs_you: 0 } },
    checkouts,
  } as unknown as Workspace;
}

/**
 * Two projects of this Mac: `alpha` holds the operator's asking agent in its main checkout and the tool's
 * session, with the child it delegated, in a worktree; `beta` holds an operator agent only. The core leaves
 * the tool's rows out of the operator's trees and publishes them in the tool's cell.
 */
function snapshot(lens: SidebarLens, view: TaskView = "status"): SnapshotRest {
  const alpha = project("alpha", [
    checkout("alpha-main", "alpha", ["mine"], [["mine", 0]], true),
    withNamedTab(checkout("alpha-tool", "alpha", ["tool-root", "tool-child"], [], false)),
  ], [prRow(7, "alpha-tool"), prRow(8, "alpha-main")], [pull(7, "alpha-tool"), pull(8, "alpha-main")]);
  const beta = project("beta", [checkout("beta-main", "beta", ["theirs"], [["theirs", 0]], true)]);
  const listed = AGENTS.map((row, index) => ({ pane_id: row.pane_id, device_id: "local", device_label: null, remote: false, index }));
  const device: AgentScope = {
    ...emptyScope(),
    listed,
    raised: [{ group: "needs_you", shown: [{ pane_id: "mine", occurrence: 0 }], more: [] }],
    tasks: { cards: [{ root: "mine", state: "needs_you", members: ["mine"], checkouts: ["alpha-main"], recency: "" }, { root: "theirs", state: "working", members: ["theirs"], checkouts: ["beta-main"], recency: "" }], needs_you: 1, recency: "" },
  };
  const cell: ToolCell = {
    connection: TOOL.connection, name: TOOL.name, turn: 0,
    devices: [{
      device_id: "local",
      tasks: { cards: [{ root: "tool-root", state: "working", members: ["tool-root", "tool-child"], checkouts: ["alpha-tool"], recency: "" }], needs_you: 0, recency: "" },
      members: [{ pane_id: "tool-root", project_id: "alpha", checkout_id: "alpha-tool" }, { pane_id: "tool-child", project_id: "alpha", checkout_id: "alpha-tool" }],
      checkouts: [{ project_id: "alpha", checkout_id: "alpha-tool", sidebar_tree: tree([["tool-root", 0], ["tool-child", 1]]), marks: { turn: 0, going: 2, done: 0, unread: 0 } }],
    }],
  };
  const local: Device = { id: "local", label: "This Mac", kind: "local", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null, agent_scope: device };
  return {
    navigator: {
      siblings_shown: 5, devices: [local], focused_device_id: "local", workspaces: [alpha, beta], agents: AGENTS, agent_scope: { ...emptyScope(), listed }, tools: [cell],
      // The operator's conversation put to sleep in the same worktree.
      sleeping_sessions: [{ sleep_id: "sleep-1", node_id: "local", checkout_path: "/fixture/alpha-tool", kind: "claude", identity_label: "잠든 대화", group: "seen", phase: "sleeping", since_unix_ms: 1, reason: null, wake_available: true, checking: false }],
    },
    ui_state: { sidebar_lens: lens, task_view: view, left_sidebar_visible: true, sessions_expanded_agent_pane_ids: [] },
  } as unknown as SnapshotRest;
}

let cleanup: (() => Promise<void>) | null = null;
afterEach(async () => {
  await cleanup?.();
  cleanup = null;
});

async function render(rest: SnapshotRest, tool: string | null) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  const tokens = (JSON.parse(tokensText) as { tokens: Record<string, { value: number }> }).tokens;
  for (const name of ["--size-sidebar-ideal", "--size-sidebar-min", "--size-sidebar-max"]) document.documentElement.style.setProperty(name, `${tokens[name]?.value}px`);
  await clientI18n.changeLanguage("ko");
  const shell = useShellStore.getState();
  const ui = useUiStore.getState();
  useShellStore.setState({ rest, agents: AGENTS, connection: "live", focusedPaneId: null });
  useUiStore.setState({ screen: { kind: "main" }, overviewOpen: false, toolCell: tool });
  const actions = createActions(() => true);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><Sidebar actions={actions} /></TooltipProvider>));
  cleanup = async () => {
    await act(async () => root.unmount());
    container.remove();
    useShellStore.setState(shell, true);
    useUiStore.setState(ui, true);
    await clientI18n.changeLanguage("en");
    vi.unstubAllGlobals();
    for (const name of ["--size-sidebar-ideal", "--size-sidebar-min", "--size-sidebar-max"]) document.documentElement.style.removeProperty(name);
  };
  return container;
}

const all = (container: HTMLElement, selector: string, attribute: string) => [...container.querySelectorAll(selector)].map((element) => element.getAttribute(attribute));
const projects = (container: HTMLElement) => all(container, "[data-project-row]", "data-project-row");
const checkouts = (container: HTMLElement) => all(container, "[data-checkout-row]", "data-checkout-row");
const agentRows = (container: HTMLElement) => all(container, "[data-checkout-agents-open] [data-agent-open]", "data-agent-open");
const lenses = (container: HTMLElement) => all(container, "[data-sidebar-lens-option]", "data-sidebar-lens-option");

it("lists in a tool's cell the projects, checkouts and session trees of the tool alone, under the operator's lens switch, raising nothing", async () => {
  const container = await render(snapshot("projects"), TOOL.connection);
  expect(lenses(container)).toEqual(["projects", "tasks", "prs"]);
  expect(projects(container)).toEqual(["alpha"]);
  expect(checkouts(container)).toEqual(["alpha-tool"]);
  // The session with the child it delegated open under it; the operator's agent of the same project is not drawn.
  expect(agentRows(container)).toEqual(["tool-root", "tool-child"]);
  // The operator's named empty tab and sleeping conversation in that worktree are theirs, not the tool's.
  expect(container.querySelector('[data-sidebar-tab-line="alpha-tool:notes"]')).toBeNull();
  expect(container.querySelector("[data-sleeping-session]")).toBeNull();
  expect(container.querySelector("[data-raised-group]")).toBeNull();
  expect(container.textContent).not.toContain("내 차례");
  expect(container.querySelector('[data-checkout-status="alpha-tool"]')).toBeNull();
});

it("leaves the operator's cell as it was: their projects, trees and raised 내 차례, and none of the tool's rows", async () => {
  const container = await render(snapshot("projects"), null);
  expect(lenses(container)).toEqual(["projects", "tasks", "prs"]);
  expect(projects(container)).toEqual(["alpha", "beta"]);
  expect(container.textContent).toContain("내 차례");
  const drawn = all(container, "[data-agent-open]", "data-agent-open");
  expect(drawn).toContain("mine");
  expect(drawn).not.toContain("tool-root");
  expect(drawn).not.toContain("tool-child");
  // Their worktree keeps their named tab and sleeping conversation.
  expect(container.querySelector('[data-sidebar-tab-line="alpha-tool:notes"]')).not.toBeNull();
  expect(container.querySelector('[data-sleeping-session="sleep-1"]')).not.toBeNull();
});

it("groups a tool's tasks by project in the 프로젝트 view of its 태스크 list", async () => {
  const container = await render(snapshot("tasks", "project"), TOOL.connection);
  expect(container.querySelector("[data-task-view]")?.getAttribute("data-task-view")).toBe("project");
  expect(all(container, "[data-task-group]", "data-task-group")).toEqual(["alpha"]);
  expect(all(container, "[data-task-row]", "data-task-row")).toEqual(["tool-root"]);
  expect(container.querySelector("[data-sidebar-project]")).toBeNull();
});

it("lists in a tool's PR tab only the pull requests of the checkouts its sessions work in", async () => {
  const container = await render(snapshot("prs"), TOOL.connection);
  expect(all(container, "[data-pr-row]", "data-pr-row")).toEqual(["7"]);
  const operator = await (async () => {
    await cleanup?.();
    cleanup = null;
    return render(snapshot("prs"), null);
  })();
  expect(all(operator, "[data-pr-row]", "data-pr-row").sort()).toEqual(["7", "8"]);
});

it("numbers with ⌥n the list a tool's cell shows: its sessions' roots in 프로젝트, its tasks in 태스크", () => {
  const numbered = (lens: SidebarLens, toolCell: string | null) => [...sidebarNumbers({ rest: snapshot(lens), agents: AGENTS }, { raisedOpen: [], taskFoldsOpen: [], toolCell }).values()];
  expect(numbered("projects", TOOL.connection)).toEqual(["tool-root"]);
  expect(numbered("tasks", TOOL.connection)).toEqual(["tool-root"]);
  expect(numbered("projects", null)).toEqual(["mine", "theirs"]);
});
