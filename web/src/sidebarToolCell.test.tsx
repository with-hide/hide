// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, afterEach, expect, it, vi } from "vitest";
import tokensText from "../../design/tokens.json?raw";
import { emptyScope } from "../test/legacyAgentScope";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { createActions } from "./actions";
import { agentCycle } from "./areaCycle";
import { noteAreaFrame } from "./areaFrames";
import { commitCycle } from "./keyboard";
import { resetRecent } from "./recent";
import type { AgentScope, AgentTreeScope, PrBoard, ToolCell } from "./agentScope";
import { TooltipProvider } from "./components/ui/tooltip";
import { clientI18n } from "./i18n/translator";
import { sidebarNumbers } from "./numbering";
import { Sidebar } from "./sidebar";
import type { AgentRow, Checkout, Device, SidebarLens, SidebarProject, SnapshotRest, TaskView, Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore, type Screen } from "./ui";

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

/** In the wide fixture the tool also works in `beta`, where its session waits on the tool's answer. */
const TOOL_TWO = agent("tool-two", { tool: TOOL, mark: "turn", group: "needs_you", demand: "question", status_code: "question" } as Partial<AgentRow>);

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
function snapshot(lens: SidebarLens, view: TaskView = "status", chosen: SidebarProject | null = null, wide = false): SnapshotRest {
  const alpha = project("alpha", [
    checkout("alpha-main", "alpha", ["mine"], [["mine", 0]], true),
    withNamedTab(checkout("alpha-tool", "alpha", ["tool-root", "tool-child"], [], false)),
  ], [prRow(7, "alpha-tool"), prRow(8, "alpha-main")], [pull(7, "alpha-tool"), pull(8, "alpha-main")]);
  const beta = wide
    ? project("beta", [checkout("beta-main", "beta", ["theirs"], [["theirs", 0]], true), checkout("beta-tool", "beta", ["tool-two"], [], false)], [prRow(9, "beta-tool")], [pull(9, "beta-tool")])
    : project("beta", [checkout("beta-main", "beta", ["theirs"], [["theirs", 0]], true)]);
  const agents = wide ? [...AGENTS, TOOL_TWO] : AGENTS;
  const listed = agents.map((row, index) => ({ pane_id: row.pane_id, device_id: "local", device_label: null, remote: false, index }));
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
  if (wide) {
    const own = cell.devices[0]!;
    own.tasks = { cards: [...own.tasks.cards, { root: "tool-two", state: "needs_you", members: ["tool-two"], checkouts: ["beta-tool"], recency: "" }], needs_you: 1, recency: "" };
    own.members = [...own.members, { pane_id: "tool-two", project_id: "beta", checkout_id: "beta-tool" }];
    own.checkouts = [...own.checkouts!, { project_id: "beta", checkout_id: "beta-tool", sidebar_tree: tree([["tool-two", 0]]), marks: { turn: 1, going: 0, done: 0, unread: 0 } }];
  }
  const local: Device = { id: "local", label: "This Mac", kind: "local", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null, agent_scope: device };
  return {
    navigator: {
      siblings_shown: 5, devices: [local], focused_device_id: "local", workspaces: [alpha, beta], agents, agent_scope: { ...emptyScope(), listed }, tools: [cell],
      // The operator's conversation put to sleep in the same worktree.
      sleeping_sessions: [{ sleep_id: "sleep-1", node_id: "local", checkout_path: "/fixture/alpha-tool", kind: "claude", identity_label: "잠든 대화", group: "seen", phase: "sleeping", since_unix_ms: 1, reason: null, wake_available: true, checking: false }],
    },
    ui_state: { sidebar_lens: lens, task_view: view, left_sidebar_visible: true, sessions_expanded_agent_pane_ids: [], sidebar_project: chosen },
  } as unknown as SnapshotRest;
}

let cleanup: (() => Promise<void>) | null = null;
let sent: { kind: string; payload: unknown }[] = [];
let drawnActions: ReturnType<typeof createActions> | null = null;
afterEach(async () => {
  await cleanup?.();
  cleanup = null;
});

/** Draws the sidebar over `screen`; a tool's cell is in front while the page names the tool, as its tile opens it. */
async function render(rest: SnapshotRest, tool: string | null, screen: Screen = tool === null ? { kind: "main" } : { kind: "main", tool }) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  const tokens = (JSON.parse(tokensText) as { tokens: Record<string, { value: number }> }).tokens;
  for (const name of ["--size-sidebar-ideal", "--size-sidebar-min", "--size-sidebar-max"]) document.documentElement.style.setProperty(name, `${tokens[name]?.value}px`);
  await clientI18n.changeLanguage("ko");
  const shell = useShellStore.getState();
  const ui = useUiStore.getState();
  // The rail draws a bot's tile from the core's access section; this one works in its own space.
  const access = { machine: "local", limit: 16, pending: [], connections: [{ id: TOOL.connection, name: TOOL.name, mode: { kind: "own" as const }, program: "/bin/hoyeon", created_ms: 0, last_used_ms: 0, agent_count: 2, mission_count: 0 }] };
  useShellStore.setState({ rest, agents: rest.navigator?.agents ?? AGENTS, access, connection: "live", focusedPaneId: null });
  useUiStore.setState({ screen, overviewOpen: false });
  sent = [];
  const actions = createActions((event) => { sent.push(event as { kind: string; payload: unknown }); return true; });
  drawnActions = actions;
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
});

/** Unmounts the cell drawn last, so the test can draw another. */
async function unmount() {
  await cleanup?.();
  cleanup = null;
}

const chip = (container: HTMLElement) => container.querySelector("[data-sidebar-project]")?.getAttribute("data-sidebar-project");
const taskRows = (container: HTMLElement) => all(container, "[data-task-row]", "data-task-row");
const prRows = (container: HTMLElement) => all(container, "[data-pr-row]", "data-pr-row");

it("lists in a tool's project chip the projects the tool works in, counted from its own work, its turn muted", async () => {
  const container = await render(snapshot("tasks", "status", null, true), TOOL.connection);
  expect(chip(container)).toBe("");
  const trigger = container.querySelector<HTMLElement>("[data-sidebar-project-trigger]")!;
  expect(trigger.textContent).toBe("모든 프로젝트");
  await act(async () => { trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  const menu = document.body.querySelector("[data-sidebar-project-menu]")!;
  // `alpha` and `beta` are the tool's; the operator's own work in them is not counted.
  expect(all(menu as HTMLElement, "[data-sidebar-project-option]", "data-sidebar-project-option")).toEqual(["", "alpha", "beta"]);
  const option = (id: string) => menu.querySelector(`[data-sidebar-project-option="${id}"]`)!;
  expect(option("alpha").textContent).toBe("alpha2");
  expect(option("beta").textContent).toBe("beta봇이 답할 차례 12");
  const turn = option("beta").querySelector("[data-sidebar-project-turn]")!;
  expect(turn.getAttribute("data-sidebar-project-turn")).toBe("tool");
  expect(turn.className).not.toContain("warning");
  await act(async () => { (option("beta") as HTMLElement).click(); });
  expect(sent.filter((event) => event.kind === "sidebar_project_set").map((event) => event.payload)).toEqual([{ project: { device_id: "local", workspace_id: "beta" } }]);
});

it("narrows a tool's 태스크 and PR lists to the stored project, the tool's turn elsewhere a muted +N", async () => {
  const alpha = { device_id: "local", workspace_id: "alpha" };
  const whole = await render(snapshot("tasks", "status", null, true), TOOL.connection);
  expect(taskRows(whole).sort()).toEqual(["tool-root", "tool-two"]);
  await unmount();
  const tasks = await render(snapshot("tasks", "status", alpha, true), TOOL.connection);
  expect(chip(tasks)).toBe("alpha");
  expect(taskRows(tasks)).toEqual(["tool-root"]);
  const elsewhere = tasks.querySelector("[data-sidebar-project-elsewhere]")!;
  expect(elsewhere.textContent).toBe("+1");
  expect(elsewhere.getAttribute("data-tone")).toBe("muted");
  expect(elsewhere.className).not.toContain("warning");
  await unmount();
  const prs = await render(snapshot("prs", "status", alpha, true), TOOL.connection);
  expect(chip(prs)).toBe("alpha");
  expect(prRows(prs)).toEqual(["7"]);
  await unmount();
  const everyPr = await render(snapshot("prs", "status", null, true), TOOL.connection);
  expect(prRows(everyPr).sort()).toEqual(["7", "9"]);
});

it("leaves a tool's lists whole while the stored project is the operator's alone, and still narrows the operator's", async () => {
  const beta = { device_id: "local", workspace_id: "beta" };
  const tool = await render(snapshot("tasks", "status", beta), TOOL.connection);
  expect(chip(tool)).toBe("");
  expect(tool.querySelector("[data-sidebar-project-trigger]")?.textContent).toBe("모든 프로젝트");
  expect(taskRows(tool)).toEqual(["tool-root"]);
  await unmount();
  const toolPrs = await render(snapshot("prs", "status", beta), TOOL.connection);
  expect(chip(toolPrs)).toBe("");
  expect(prRows(toolPrs)).toEqual(["7"]);
  await unmount();
  const operator = await render(snapshot("tasks", "status", beta), null);
  expect(chip(operator)).toBe("beta");
  // The operator's 내 차례 in `alpha` stays in sight in the warning colour.
  expect(operator.querySelector("[data-sidebar-project-elsewhere]")?.getAttribute("data-tone")).toBe("warning");
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

it("numbers with ⌥n the list a tool's cell shows: its sessions' roots in 프로젝트, its tasks in 태스크, as the chip narrows them", () => {
  const numbered = (lens: SidebarLens, toolCell: string | null, chosen: SidebarProject | null = null, wide = false) => {
    const rest = snapshot(lens, "status", chosen, wide);
    return [...sidebarNumbers({ rest, agents: rest.navigator!.agents! }, { raisedOpen: [], taskFoldsOpen: [], screen: toolCell === null ? null : { kind: "main", tool: toolCell } }).values()];
  };
  expect(numbered("projects", TOOL.connection)).toEqual(["tool-root"]);
  expect(numbered("tasks", TOOL.connection)).toEqual(["tool-root"]);
  expect(numbered("projects", null)).toEqual(["mine", "theirs"]);
  expect(numbered("tasks", TOOL.connection, null, true)).toEqual(["tool-two", "tool-root"]);
  expect(numbered("tasks", TOOL.connection, { device_id: "local", workspace_id: "alpha" }, true)).toEqual(["tool-root"]);
});

/**
 * The core's answer once `paneId` is in front: its checkout focused and the
 * pane first in the order the keyboard visited panes, as the core keeps it.
 */
function inFront(rest: SnapshotRest, checkoutId: string, paneId: string): SnapshotRest {
  const recent = rest.ui_state?.recent_pane_ids ?? [];
  return {
    ...rest,
    navigator: { ...rest.navigator!, focused_checkout_id: checkoutId, focused_workspace_id: "alpha" },
    focused: { pane_id: paneId },
    ui_state: { ...rest.ui_state!, recent_pane_ids: [paneId, ...recent.filter((id) => id !== paneId)] },
  } as SnapshotRest;
}

/** The Agent area as the Workspace draws it for `checkoutId`, showing the tab of `paneId`, which the pane cycle reads. */
function drawArea(checkoutId: string, paneId: string) {
  const tab = `${checkoutId}:${paneId}`;
  noteAreaFrame("agent", { workspace: { device_id: "local", path: `/fixture/${checkoutId}` }, layout: { root: { area: { id: "a1", active: tab, displays: [{ id: tab }] } }, canvases: {} }, geometry: { areas: [{ id: "a1" }] } } as never);
}

/** ⌃Tab once from the pane the keyboard is in, released at once, as the keyboard's cycle does it. */
async function cycleOnce(checkoutId: string, paneId: string) {
  drawArea(checkoutId, paneId);
  const cycle = agentCycle(useShellStore.getState().rest, { kind: "pane", workspace: checkoutId, paneId })!;
  await act(async () => { commitCycle({ ...cycle, index: (cycle.index + 1) % cycle.items.length }, drawnActions!); });
}

/** The pane the last `focus_pane` asked for. */
const focusAsked = () => (sent.filter((event) => event.kind === "focus_pane").at(-1)?.payload as { pane_id: string } | undefined)?.pane_id;

/** The core moves the focus where it was asked, and the page shows the Workspace once that open landed (`CenterScreen`). */
async function coreFocuses(checkoutId: string, paneId: string) {
  await act(async () => {
    useShellStore.setState((state) => ({ rest: inFront(state.rest!, checkoutId, paneId), focusedPaneId: paneId }));
    useUiStore.getState().setScreen({ kind: "workspace" });
  });
}

const railSelected = (container: HTMLElement) => [...container.querySelectorAll('[data-device-rail] [aria-pressed="true"]')].map((tile) => tile.getAttribute("data-rail-tile") ?? `tool:${tile.getAttribute("data-rail-tool")}`);
const listCell = (container: HTMLElement) => container.querySelector("[data-sidebar-content] [data-tool-cell]")?.getAttribute("data-tool-cell") ?? null;

// PRD hide-external-control B40 as the operator found it (2026-11-15): a tool's cell is a scope like a device's,
// so landing on another pane by any path moves the rail and the lists with it, as landing on a device's pane does.
it("moves the rail and the lists with ⌃Tab as a device's do: off a tool's session onto the operator's pane, and back onto the tool's", async () => {
  resetRecent();
  noteAreaFrame("agent", null);
  try {
    // The operator was in their own pane, then pressed the tool's tile and its session's row.
    const container = await render(inFront(snapshot("projects"), "alpha-main", "mine"), null, { kind: "workspace" });
    expect(railSelected(container)).toEqual(["local"]);
    await act(async () => container.querySelector<HTMLElement>(`[data-rail-tool="${TOOL.connection}"]`)!.click());
    expect(railSelected(container)).toEqual([`tool:${TOOL.connection}`]);
    expect(listCell(container)).toBe(TOOL.connection);
    await act(async () => container.querySelector<HTMLElement>('[data-agent-open="tool-root"]')!.click());
    expect(focusAsked()).toBe("tool-root");
    await coreFocuses("alpha-tool", "tool-root");
    expect(railSelected(container)).toEqual([`tool:${TOOL.connection}`]);
    expect(listCell(container)).toBe(TOOL.connection);

    // ⌃Tab lands on the operator's pane: the rail selects the device and the sidebar lists the operator's work.
    await cycleOnce("alpha-tool", "tool-root");
    expect(focusAsked()).toBe("mine");
    await coreFocuses("alpha-main", "mine");
    expect(railSelected(container)).toEqual(["local"]);
    expect(listCell(container)).toBeNull();
    expect(projects(container)).toEqual(["alpha", "beta"]);

    // ⌃Tab back onto the tool's session selects the tool's tile, as landing on a device's pane selects that device.
    await cycleOnce("alpha-main", "mine");
    expect(focusAsked()).toBe("tool-root");
    await coreFocuses("alpha-tool", "tool-root");
    expect(railSelected(container)).toEqual([`tool:${TOOL.connection}`]);
    expect(listCell(container)).toBe(TOOL.connection);
    expect(projects(container)).toEqual(["alpha"]);
  } finally {
    noteAreaFrame("agent", null);
  }
});

it("takes the window off a tool's session onto the device with the device's tile, landing on the operator's last pane there", async () => {
  resetRecent();
  const container = await render(inFront(inFront(snapshot("projects"), "alpha-main", "mine"), "alpha-tool", "tool-root"), null, { kind: "workspace" });
  expect(railSelected(container)).toEqual([`tool:${TOOL.connection}`]);
  // This Mac is in front already, under the tool's session: the press brings back the pane the operator was last in.
  await act(async () => container.querySelector<HTMLElement>('[data-rail-tile="local"]')!.click());
  expect(focusAsked()).toBe("mine");
  await coreFocuses("alpha-main", "mine");
  expect(railSelected(container)).toEqual(["local"]);
  expect(listCell(container)).toBeNull();
});

it("opens from a tool's checkout row the tool's session there when the checkout shows the operator's tab, so the press stays in the cell", async () => {
  resetRecent();
  const rest = snapshot("projects");
  // The operator's named tab is the one the worktree shows.
  const alpha = rest.navigator!.workspaces!.find((row) => row.id === "alpha")!;
  alpha.checkouts = alpha.checkouts.map((row) => (row.id === "alpha-tool" ? { ...row, active_tab_id: "alpha-tool:notes" } : row));
  const container = await render(rest, TOOL.connection);
  await act(async () => container.querySelector<HTMLElement>('[data-checkout="alpha-tool"]')!.click());
  expect(sent.filter((event) => event.kind === "focus_checkout")).toEqual([]);
  expect(focusAsked()).toBe("tool-root");
});

// B42 and the operator's 2026-11-15 capture: the usage footer stays at the bottom in an empty tool cell. The slot the
// lists stand in fills the column above the footer, so whatever a list draws, empty or full, the footer cannot ride up.
it("keeps the footer under a list slot that fills the column, in every list of an empty tool's cell and of the operator's", async () => {
  const empty = (rest: SnapshotRest): SnapshotRest => ({ ...rest, navigator: { ...rest.navigator!, tools: [{ connection: TOOL.connection, name: TOOL.name, turn: 0, devices: [] }] } }) as SnapshotRest;
  for (const [lens, tool] of [["projects", TOOL.connection], ["tasks", TOOL.connection], ["prs", TOOL.connection], ["projects", null], ["tasks", null]] as const) {
    const container = await render(empty(snapshot(lens)), tool);
    const content = container.querySelector<HTMLElement>("[data-sidebar-content]")!;
    const footer = content.querySelector<HTMLElement>(":scope > [data-sidebar-footer]")!;
    // The child of the column that holds the list grows to fill it, and the footer comes after it.
    const slot = [...content.children].find((child) => child !== footer && child.contains(tool ? container.querySelector("[data-tool-cell-empty]") : container.querySelector(lens === "tasks" ? "[data-task-lens]" : "[data-project-list]")));
    expect(slot, `${lens} ${tool}`).toBeDefined();
    expect(slot!.classList.contains("flex-1") && slot!.classList.contains("min-h-0"), `${lens} ${tool}`).toBe(true);
    expect(slot!.compareDocumentPosition(footer) & Node.DOCUMENT_POSITION_FOLLOWING, `${lens} ${tool}`).toBeTruthy();
    if (tool) expect(container.querySelector("[data-tool-cell-empty]")?.textContent, lens).toBe("아직 맡은 일이 없어요");
    await unmount();
  }
});
