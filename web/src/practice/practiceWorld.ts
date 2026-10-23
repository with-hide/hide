// The practice window's world: one invented project the real shell draws,
// and the few core events a chapter asks for, applied here the way the core
// would apply them. Nothing leaves the page: no daemon, no Herdr, no agent.

import type { AgentGroup, AgentLayout } from "../agentLayout";
import type { AgentScope } from "../agentScope";
import type { AgentRow, AsyncOperation, Checkout, LayoutNode, SnapshotRest } from "../snapshot";
import type { PaneMoveIntent } from "../paneMove";
import { galleryAgentState } from "../gallery/agentStates";
import type { CoreEvent, PracticeWorld } from "./world";
import { agent, checkout, REFERENCE_FOLDS, sidebarStatusAxisScene, workspace } from "../gallery/sceneData";
import statusAxisScope from "../gallery/sidebarStatusAxisScope.json";

export const PROJECT = "hide-practice";
export const PROJECT_LABEL = "hide-practice";
const CHECKOUT = `${PROJECT}:main`;

type PracticeTab = { id: string; name: string | null; root: LayoutNode; focused: string; zoomed: boolean };
type PracticePane = { id: string; agent: AgentRow | null; title: string };

/** What a chapter can change: tabs, panes, groups and which one is in front. */
export type PracticeModel = {
  panes: PracticePane[];
  tabs: PracticeTab[];
  groups: AgentGroup[];
  selectedGroup: string | null;
  activeTab: string;
  nextId: number;
  /** The pane moves the core finished, as `status.async_operations` reports them. */
  operations: AsyncOperation[];
};

/** The world chapter 1 opens on: one Claude agent at work in the practice project's only tab. */
export function chapterOneModel(): PracticeModel {
  const lead = agent({ pane_id: "lead", identity_label: "Fix the checkout 500", agent_kind: "claude", group: "working", mark: "going", status_code: "working", activity: "working", elapsed: "1m", state: galleryAgentState("e1", null, Date.now() - 60_000) });
  return {
    panes: [{ id: "lead", agent: lead, title: "Fix the checkout 500" }],
    tabs: [{ id: "t1", name: null, root: { type: "pane", pane_id: "lead" }, focused: "lead", zoomed: false }],
    groups: [{ id: null, name: null, tabs: ["t1"] }],
    selectedGroup: null,
    activeTab: "t1",
    nextId: 2,
    operations: [],
  };
}

const panesOf = (node: LayoutNode): string[] => (node.type === "pane" ? [node.pane_id] : [...panesOf(node.first), ...panesOf(node.second)]);

/** `node` without the pane `id`; a split left with one side becomes that side. */
function withoutPane(node: LayoutNode, id: string): LayoutNode | null {
  if (node.type === "pane") return node.pane_id === id ? null : node;
  const first = withoutPane(node.first, id);
  const second = withoutPane(node.second, id);
  if (!first) return second;
  if (!second) return first;
  return { ...node, first, second };
}

/** The pane `id` placed on `edge` of the pane `beside`. */
function placeBeside(node: LayoutNode, beside: string, id: string, edge: "left" | "right" | "up" | "down"): LayoutNode {
  const moved: LayoutNode = { type: "pane", pane_id: id };
  const there: LayoutNode = { type: "pane", pane_id: beside };
  const before = edge === "left" || edge === "up";
  return replacePane(node, beside, { type: "split", direction: edge === "left" || edge === "right" ? "right" : "down", ratio: 0.5, first: before ? moved : there, second: before ? there : moved });
}

function replacePane(node: LayoutNode, id: string, next: LayoutNode): LayoutNode {
  if (node.type === "pane") return node.pane_id === id ? next : node;
  return { ...node, first: replacePane(node.first, id, next), second: replacePane(node.second, id, next) };
}

function tabOfPane(model: PracticeModel, paneId: string): PracticeTab | undefined {
  return model.tabs.find((tab) => panesOf(tab.root).includes(paneId));
}

function groupOfTab(model: PracticeModel, tabId: string): AgentGroup {
  const group = model.groups.find((row) => row.tabs.includes(tabId));
  if (!group) throw new Error(`Practice tab ${tabId} belongs to no group`);
  return group;
}

/** A plain shell: what a split or a new tab opens. */
function shellPane(model: PracticeModel): PracticePane {
  return { id: `sh${model.nextId}`, agent: null, title: "zsh" };
}


/**
 * The model after `event`, as the core would leave it, or null for an event
 * a chapter never needs: the practice window answers only what it teaches.
 */
export function applyPractice(model: PracticeModel, event: CoreEvent): PracticeModel | null {
  const p = event.payload;
  switch (event.kind) {
    case "create_pane": {
      const tab = model.tabs.find((row) => row.id === p.tab_id);
      if (!tab) return null;
      const pane = shellPane(model);
      const root = replacePane(tab.root, tab.focused, { type: "split", direction: p.direction === "down" ? "down" : "right", ratio: 0.5, first: { type: "pane", pane_id: tab.focused }, second: { type: "pane", pane_id: pane.id } });
      return { ...model, panes: [...model.panes, pane], nextId: model.nextId + 1, tabs: model.tabs.map((row) => (row === tab ? { ...row, root, focused: pane.id, zoomed: false } : row)) };
    }
    case "focus_pane": {
      const id = String(p.pane_id);
      const tab = tabOfPane(model, id);
      if (!tab) return null;
      const group = groupOfTab(model, tab.id);
      return { ...model, activeTab: tab.id, selectedGroup: group.id, tabs: model.tabs.map((row) => (row === tab ? { ...row, focused: id } : row)) };
    }
    case "toggle_zoom": {
      const tab = tabOfPane(model, String(p.pane_id));
      if (!tab) return null;
      return { ...model, tabs: model.tabs.map((row) => (row === tab ? { ...row, focused: String(p.pane_id), zoomed: !row.zoomed } : row)) };
    }
    case "pane_move":
      return movePane(model, p as unknown as PaneMoveIntent);
    case "agent_group":
      return applyGroup(model, p.group_id as string | null, p);
    case "agent_layout":
      if (p.action === "focus" && typeof p.tab_id === "string") return { ...model, activeTab: p.tab_id };
      return null;
    case "focus_tab":
      if (typeof p.tab_id === "string") return { ...model, activeTab: p.tab_id };
      return null;
    default:
      return null;
  }
}

/** A pane moved the way the core moves it, with the finished operation the shell waits for. */
function movePane(model: PracticeModel, intent: PaneMoveIntent): PracticeModel | null {
  const { source, destination } = intent;
  const from = model.tabs.find((tab) => tab.id === source.tab_id);
  if (!from) return null;
  const rest = withoutPane(from.root, source.pane_id);
  let tabs = model.tabs.map((tab) => (tab === from ? { ...tab, root: rest ?? tab.root, focused: rest && tab.focused === source.pane_id ? panesOf(rest)[0]! : tab.focused } : tab));
  let groups = model.groups;
  let activeTab: string;
  let nextId = model.nextId;
  if (destination.type === "new_tab") {
    if (!rest) return null;
    const id = `t${nextId++}`;
    tabs = [...tabs, { id, name: null, root: { type: "pane", pane_id: source.pane_id }, focused: source.pane_id, zoomed: false }];
    groups = groups.map((group) => (group.tabs.includes(from.id) ? { ...group, tabs: [...group.tabs, id] } : group));
    activeTab = id;
  } else {
    const edge = destination.type === "pane" ? destination.edge : "right";
    tabs = tabs.map((tab) => (tab.id === destination.tab_id ? { ...tab, root: placeBeside(tab.id === from.id ? rest! : tab.root, destination.pane_id, source.pane_id, edge), focused: source.pane_id, zoomed: false } : tab));
    activeTab = destination.tab_id;
  }
  if (!rest) {
    tabs = tabs.filter((tab) => tab.id !== from.id);
    groups = groups.map((group) => ({ ...group, tabs: group.tabs.filter((tab) => tab !== from.id) }));
  }
  const done: AsyncOperation = { id: intent.request_id, kind: "pane.move", target_id: "local", scope_id: source.tab_id, phase: "completed", stage: "completed", message: null, retryable: false };
  return { ...model, tabs, groups, activeTab, nextId, operations: [...model.operations, done] };
}

function applyGroup(model: PracticeModel, groupId: string | null, p: Record<string, unknown>): PracticeModel | null {
  switch (p.action) {
    case "new_tab": {
      const pane = shellPane(model);
      const tab: PracticeTab = { id: `t${model.nextId}`, name: null, root: { type: "pane", pane_id: pane.id }, focused: pane.id, zoomed: false };
      return { ...model, panes: [...model.panes, pane], tabs: [...model.tabs, tab], nextId: model.nextId + 1, activeTab: tab.id, groups: model.groups.map((row) => (row.id === groupId ? { ...row, tabs: [...row.tabs, tab.id] } : row)) };
    }
    case "select": {
      const group = model.groups.find((row) => row.id === groupId);
      if (!group) return null;
      const tabId = typeof p.tab_id === "string" ? p.tab_id : group.tabs.includes(model.activeTab) ? model.activeTab : group.tabs[0];
      return { ...model, selectedGroup: groupId, activeTab: tabId ?? model.activeTab };
    }
    case "move": {
      const tabId = String(p.tab_id);
      return { ...model, groups: model.groups.map((row) => ({ ...row, tabs: row.id === groupId ? [...row.tabs.filter((tab) => tab !== tabId), tabId] : row.tabs.filter((tab) => tab !== tabId) })) };
    }
    default:
      return null;
  }
}

/** The pane holding the keyboard: the front tab's focused pane. */
export function focusedPane(model: PracticeModel): string {
  const tab = model.tabs.find((row) => row.id === model.activeTab);
  if (!tab) throw new Error(`Practice front tab ${model.activeTab} is missing`);
  return tab.focused;
}

/** The core's scope for agents that are all roots in one checkout, which is every practice chapter's. */
function flatScope(ids: string[]): AgentScope {
  const scope = structuredClone(statusAxisScope) as unknown as AgentScope & Record<string, unknown>;
  const refs = ids.map((pane_id) => ({ pane_id, occurrence: 0 }));
  const tree = { rows: refs.map((ref) => ({ ...ref, depth: 0 })), visible_rows: refs.map((ref) => ({ ...ref, depth: 0 })), shown: ids, more: 0, needs_you: false, turn_kind: null };
  return {
    ...scope,
    listed: ids.map((pane_id, index) => ({ pane_id, device_id: "local", device_label: null, remote: false, index })),
    places: Object.fromEntries(ids.map((id) => [id, { project_id: PROJECT, checkout_id: CHECKOUT, kind: "checkout" as const }])),
    members: ids.map((pane_id) => ({ pane_id, project_id: PROJECT, checkout_id: CHECKOUT })),
    rows: refs,
    roots: refs,
    tree,
    sidebar_tree: tree,
    global_tree: tree,
    total: ids.length,
    overview_total: ids.length,
    badge_total: ids.length,
    groups: { needs_you: 0, done: 0, working: ids.length, seen: 0 },
    marks: { turn: 0, going: ids.length, done: 0, unread: 0 },
  };
}

/** The snapshot the core would send for `model`, built on the sidebar scene's device and settings. */
export function practiceSnapshot(model: PracticeModel, nowMs: number): { rest: SnapshotRest; agents: AgentRow[]; focused: string } {
  const base = sidebarStatusAxisScene("reference", REFERENCE_FOLDS, nowMs);
  const agents = model.panes.flatMap((pane) => (pane.agent ? [pane.agent] : []));
  const scope = flatScope(agents.map((row) => row.pane_id));
  const main: Checkout = checkout({ id: CHECKOUT, workspace: PROJECT, branch: "main", primary: true, age: 600, panes: model.panes.map((pane) => pane.id), marks: { going: agents.length } }, nowMs / 1000);
  const template = main.tabs[0]!;
  const titled = (id: string) => {
    const pane = model.panes.find((row) => row.id === id)!;
    return { ...template.panes.find((row) => row.id === id)!, identity_label: pane.title, terminal_title: pane.agent ? null : pane.title, cwd: main.path };
  };
  main.tabs = model.tabs.map((tab, index) => ({ ...template, id: tab.id, label: String(index + 1), sidebar: { number: index + 1, manual_name: tab.name, named_by_operator: tab.name !== null }, panes: panesOf(tab.root).map(titled) }));
  main.active_tab_id = model.activeTab;
  main.strip = main.tabs.map((tab) => ({ id: `strip:${tab.id}`, kind: "herdr", source_id: tab.id!, label: tab.sidebar?.manual_name ?? `Tab ${tab.sidebar?.number}`, preview: false }));
  main.next_tab_label = String(model.tabs.length + 1);
  main.agent_groups = { selected: model.selectedGroup, groups: model.groups, drafts: [] };
  main.agent_scope = scope;
  const project = { ...workspace(PROJECT, { checkouts: [main], agent_scope: scope, agent_presence: { working: true } }, REFERENCE_FOLDS), label: PROJECT_LABEL };
  const shown = model.groups.find((row) => row.id === model.selectedGroup)?.tabs ?? [];
  const groupFront = shown.includes(model.activeTab) ? model.activeTab : (shown[0] ?? null);
  const agentLayout: AgentLayout = {
    root: { area: { id: "a1", active: groupFront, displays: shown.map((id) => ({ id })) } },
    active_area: "a1", display_count: shown.length, limits: { areas: 6, depth: 3, displays: 64 }, canvases: {},
    group_id: model.selectedGroup, groups: model.groups, drafts: [], can_undo_group: false,
  };
  const focused = focusedPane(model);
  const navigator = base.rest.navigator!;
  const rest: SnapshotRest = {
    ...base.rest,
    navigator: {
      ...navigator,
      agent_scope: scope,
      devices: [{ ...navigator.devices![0]!, agent_scope: scope, agent_count: agents.length }],
      workspaces: [project],
      agents,
      focused_workspace_id: PROJECT,
      focused_checkout_id: CHECKOUT,
      focused_device_id: "local",
      sleeping_sessions: [],
      inactive_projects: [],
      no_agent_projects: [],
    },
    ui_state: {
      ...base.rest.ui_state!,
      sidebar_lens: "projects",
      sidebar_width: 280,
      device_rail_visible: false,
      left_sidebar_visible: true,
      workspace_registrations: [{ device_id: "local", path: project.path, label: PROJECT_LABEL } as never],
    },
    focused: { pane_id: focused },
    status: { ...base.rest.status!, async_operations: model.operations },
    pane_layouts: model.tabs.map((tab) => ({ workspace_id: PROJECT, tab_id: tab.id, focused_pane_id: tab.focused, zoomed: tab.zoomed, root: tab.root })),
    workspace_view: {
      device_id: "local", path: main.path, open: false, documents: false, tool: null, pinned: false,
      document_width: null, detail_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0, lane_called_side: "view",
      layout: { root: { area: { id: "v1", active: null, displays: [] } }, active_area: "v1", display_count: 0, limits: { areas: 4, depth: 3, displays: 32 } },
      agent_layout: agentLayout,
    },
    terminal: { headers: Object.fromEntries(agents.map((row) => [row.pane_id, { working: true, notice: null, band: null }])) },
  };
  return { rest, agents, focused };
}

/** What each pane's terminal shows when it first appears. */
export function paneScreen(pane: PracticePane): string {
  if (!pane.agent) return "\x1b[32m~/hide-practice\x1b[0m \x1b[2mmain\x1b[0m\r\n$ ";
  return [
    "\x1b[38;5;209m✻\x1b[0m Claude Code",
    "",
    "\x1b[2m>\x1b[0m Checkout returns a 500 when I pay. Fix it",
    "",
    "\x1b[38;5;209m●\x1b[0m Following the payment request through the code.",
    "  \x1b[2m⎿  Reading src/payment/checkout.ts…\x1b[0m",
    "",
    "\x1b[38;5;209m✻\x1b[0m \x1b[2mLooking for the cause… (esc to interrupt)\x1b[0m",
  ].join("\r\n");
}

/** Chapter 1's world: the practice project, answering splits, focus, zoom, tabs and groups. */
export const PRACTICE_WORLD: PracticeWorld<PracticeModel> = {
  initial: chapterOneModel,
  apply: applyPractice,
  snapshot: (model, nowMs) => ({ ...practiceSnapshot(model, nowMs), usage: null }),
  panes: (model) => model.panes.map((pane) => pane.id),
  screen: (model, paneId) => paneScreen(model.panes.find((pane) => pane.id === paneId)!),
  lane: false,
};
