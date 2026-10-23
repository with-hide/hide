// The tools practice worlds are built with: a checkout's desk of tabs, panes
// and groups, the core events that change it, applied the way the core
// applies them, and the snapshot the core would send for it. Every value here
// is invented for the practice screen; nothing names a person, a machine or a
// path of anyone's.

import type { AgentGroup, AgentLayout } from "../../agentLayout";
import type { AgentScope, AgentTreeScope } from "../../agentScope";
import type { PaneMoveIntent } from "../../paneMove";
import type { AgentRow, AgentState, AsyncOperation, Checkout, LayoutNode, SnapshotRest, Workspace } from "../../snapshot";
import type { CoreEvent, WorldSnapshot } from "../world";

export type PracticeTab = { id: string; name: string | null; root: LayoutNode; focused: string; zoomed: boolean };
/** A pane, and the agent running in it; a pane with no agent is a plain shell. */
export type PracticePane = { id: string; agent: AgentRow | null; title: string };

/** What a chapter can change on one checkout: its panes, tabs and groups, and which one is in front. */
export type DeskModel = {
  panes: PracticePane[];
  tabs: PracticeTab[];
  groups: AgentGroup[];
  selectedGroup: string | null;
  activeTab: string;
  nextId: number;
  /** The pane moves the core finished, as `status.async_operations` reports them. */
  operations: AsyncOperation[];
};

/** Where a desk sits: its project, and the branch of the project's main checkout. */
export type PracticePlace = { project: string; label: string; branch: string };

/** One project on the practice screen: its place and the desk on its main checkout. */
export type PracticeDesk = { place: PracticePlace; model: DeskModel };

/** Every practice project's folder is under this invented root. */
const ROOT = "/work";

const projectPath = (place: PracticePlace) => `${ROOT}/${place.project}`;
const checkoutId = (place: PracticePlace) => `${place.project}:${place.branch}`;

// --- The layout tree -------------------------------------------------------

export const panesOf = (node: LayoutNode): string[] => (node.type === "pane" ? [node.pane_id] : [...panesOf(node.first), ...panesOf(node.second)]);

/** `node` without the pane `id`; a split left with one side becomes that side. */
function withoutPane(node: LayoutNode, id: string): LayoutNode | null {
  if (node.type === "pane") return node.pane_id === id ? null : node;
  const first = withoutPane(node.first, id);
  const second = withoutPane(node.second, id);
  if (!first) return second;
  if (!second) return first;
  return { ...node, first, second };
}

function replacePane(node: LayoutNode, id: string, next: LayoutNode): LayoutNode {
  if (node.type === "pane") return node.pane_id === id ? next : node;
  return { ...node, first: replacePane(node.first, id, next), second: replacePane(node.second, id, next) };
}

/** The pane `id` placed on `edge` of the pane `beside`. */
function placeBeside(node: LayoutNode, beside: string, id: string, edge: "left" | "right" | "up" | "down"): LayoutNode {
  const moved: LayoutNode = { type: "pane", pane_id: id };
  const there: LayoutNode = { type: "pane", pane_id: beside };
  const before = edge === "left" || edge === "up";
  return replacePane(node, beside, { type: "split", direction: edge === "left" || edge === "right" ? "right" : "down", ratio: 0.5, first: before ? moved : there, second: before ? there : moved });
}

// --- Tabs, groups and focus ------------------------------------------------

export function tabById(model: DeskModel, id: string): PracticeTab {
  const tab = model.tabs.find((row) => row.id === id);
  if (!tab) throw new Error(`Practice tab ${id} is missing`);
  return tab;
}

function tabOfPane(model: DeskModel, paneId: string): PracticeTab | undefined {
  return model.tabs.find((tab) => panesOf(tab.root).includes(paneId));
}

function groupOfTab(model: DeskModel, tabId: string): AgentGroup {
  const group = model.groups.find((row) => row.tabs.includes(tabId));
  if (!group) throw new Error(`Practice tab ${tabId} belongs to no group`);
  return group;
}

/** The pane holding the keyboard: the front tab's focused pane. */
export function focusedPane(model: DeskModel): string {
  const tab = model.tabs.find((row) => row.id === model.activeTab);
  if (!tab) throw new Error(`Practice front tab ${model.activeTab} is missing`);
  return tab.focused;
}

/** A desk of one tab in the ungrouped group, holding `pane` alone. */
export function oneTabDesk(pane: PracticePane): DeskModel {
  return {
    panes: [pane],
    tabs: [{ id: "t1", name: null, root: { type: "pane", pane_id: pane.id }, focused: pane.id, zoomed: false }],
    groups: [{ id: null, name: null, tabs: ["t1"] }],
    selectedGroup: null,
    activeTab: "t1",
    nextId: 2,
    operations: [],
  };
}

/** A plain shell: what a split or a new tab opens. */
function shellPane(model: DeskModel): PracticePane {
  return { id: `sh${model.nextId}`, agent: null, title: "zsh" };
}

/** What a plain shell's terminal shows: the prompt in the desk's project. */
export function shellScreen(place: PracticePlace): string {
  return `\x1b[32m~/${place.project}\x1b[0m \x1b[2m${place.branch}\x1b[0m\r\n$ `;
}

// --- An agent's prompt --------------------------------------------------------

/** The text a terminal's `key` event carries (`bytes_base64`). */
export const keyText = (base64: string) => new TextDecoder().decode(Uint8Array.from(atob(base64), (char) => char.charCodeAt(0)));

/**
 * The prompt after the keys `text`, as an agent's input line takes them: ⌃U
 * clears it, ⌫ takes a character back, ⏎ hands it to `submit`; an arrow or
 * other escape sequence moves nothing.
 */
export function typed<M extends { draft: string }>(model: M, text: string, submit: (model: M) => M): M {
  let next = model;
  let escape = false;
  for (const char of text) {
    if (escape) escape = !/[A-Za-z~]/.test(char);
    else if (char === "\x1b") escape = true;
    else if (char === "\r") next = submit(next);
    else if (char === "\x15") next = { ...next, draft: "" };
    else if (char === "\x7f") next = { ...next, draft: [...next.draft].slice(0, -1).join("") };
    else if (char >= " ") next = { ...next, draft: next.draft + char };
  }
  return next;
}

// --- The core events a desk answers -----------------------------------------

/**
 * The desk after `event`, as the core would leave it, or null for an event
 * the desk does not answer: splits, focus, zoom, pane moves, tabs and groups.
 */
export function applyDesk(model: DeskModel, event: CoreEvent): DeskModel | null {
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
function movePane(model: DeskModel, intent: PaneMoveIntent): DeskModel | null {
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

function applyGroup(model: DeskModel, groupId: string | null, p: Record<string, unknown>): DeskModel | null {
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

// --- Agent rows -------------------------------------------------------------

/** How a working root agent reads, as the core states it, since `since`. */
function workingState(since: number): AgentState {
  return {
    attention: false, needs_you: false, root: true, title_emphasized: false, selection_emphasizes_title: true,
    asking: false, working: true, waits_on_children: false,
    chip_tone: { kind: "working", read: false }, mark_tone: { kind: "working", read: false },
    line: null, branch_badge: null, bucket: "working", attention_rank: 3, link_rank: 1, search_tone: "working", subtree: "working", link: "working",
    descendant_asking: 0, verb: "working", request_todo: false, request_since: since,
    session: { group: "working", line: null, unfinished: false }, ask: null, tree_rank: 1, pr: null,
  };
}

const UNIT_MS: Record<string, number> = { s: 1_000, m: 60_000, h: 3_600_000, d: 86_400_000 };

/**
 * An agent row as the core sends it, changed `elapsed` ago (`42s`, `3m`, `2h`).
 * It is a working root unless the spec says otherwise; a child names its
 * parent the way the core does (`lineage_parent_pane_id`).
 */
export function agent({ elapsed, ...spec }: Partial<AgentRow> & Pick<AgentRow, "pane_id" | "identity_label" | "group" | "mark" | "status_code"> & { elapsed: string }): AgentRow {
  const changed = Date.now() - Number(elapsed.slice(0, -1)) * UNIT_MS[elapsed.slice(-1)]!;
  return {
    state: workingState(changed),
    id: spec.pane_id,
    agent_kind: "claude",
    // The core draws a row bright exactly while it is in Needs You or Done.
    emphasized: spec.group === "needs_you" || spec.group === "done",
    unread: false,
    demand: "none",
    activity: "idle",
    changed_at_unix_ms: changed,
    ...spec,
  };
}

// --- The core's scope and the snapshot ----------------------------------------

const tree = (ids: string[]) => {
  const rows = ids.map((pane_id) => ({ pane_id, occurrence: 0, depth: 0 }));
  return { rows, visible_rows: rows, shown: ids, more: 0, needs_you: false, turn_kind: null };
};

/** The core's scope for working agents that are all roots, each in its desk's checkout. */
function rootScope(members: { pane_id: string; place: PracticePlace }[]): AgentScope {
  const ids = members.map((member) => member.pane_id);
  const refs = ids.map((pane_id) => ({ pane_id, occurrence: 0 }));
  return {
    overview_needs_you: 0,
    has_working: ids.length > 0,
    relations: {},
    listed: ids.map((pane_id, index) => ({ pane_id, device_id: "local", device_label: null, remote: false, index })),
    places: Object.fromEntries(members.map(({ pane_id, place }) => [pane_id, { project_id: place.project, checkout_id: checkoutId(place), kind: "checkout" as const }])),
    places_live: true,
    closes: {},
    raised: [],
    owners: {},
    badge_total: ids.length,
    prs: { rows: [], groups: [], needs_you: 0 },
    rows: refs,
    total: ids.length,
    overview_total: ids.length,
    roots: refs,
    groups: { needs_you: 0, done: 0, working: ids.length, seen: 0 },
    marks: { turn: 0, going: ids.length, done: 0, unread: 0 },
    group_rows: [],
    descendants: {},
    children: {},
    sections: [],
    members: members.map(({ pane_id, place }) => ({ pane_id, project_id: place.project, checkout_id: checkoutId(place) })),
    buckets: { turn: 0, working: ids.length, delegating: 0, resting: 0 },
    turns: { question: 0, approval: 0, error: 0, done: 0 },
    folded: {},
    tree: tree(ids),
    sidebar_tree: tree(ids),
    global_tree: tree(ids),
    requests: { rows: [], groups: [], counts: { answer: 0, fix: 0, review: 0, blocked: 0, stopped: 0, result: 0, working: 0, waiting: 0, idle: 0 }, todo: 0, answer: 0 },
  };
}

/** Whether a tab is somebody else's work, as the core decides it: it holds an agent, and every agent in it is a delegated child. */
function tabIsDelegated(model: DeskModel, tab: PracticeTab): boolean {
  const agents = panesOf(tab.root).flatMap((id) => model.panes.find((pane) => pane.id === id)?.agent ?? []);
  return agents.length > 0 && agents.every((row) => row.delegated === true);
}

/**
 * What closing a pane, a tab or a whole desk would end, keyed by its pane ids
 * the way the core keys them (`tally/close.rs`). A practice agent tracks no
 * descendants to close with it, so no close asks about a subtree.
 */
function closes(desks: readonly PracticeDesk[]): AgentScope["closes"] {
  const result: AgentScope["closes"] = {};
  const add = (ids: string[], panes: PracticePane[]) => {
    const asks = (pane: PracticePane) => pane.agent?.requires_close_confirmation === true;
    result[ids.join("\0")] ??= {
      decision: { action: panes.some(asks) ? "confirm" : "close" },
      stop_work: { rows: panes.map((pane) => ({ pane_id: pane.id, agent: pane.agent !== null, label: pane.agent?.identity_label ?? pane.title, state: asks(pane) ? "active" : "quiet" })), unknown: null },
      subtree: null,
      subtree_all: null,
    };
  };
  add([], []);
  for (const { model } of desks) {
    const paneOf = (id: string) => model.panes.find((pane) => pane.id === id)!;
    for (const tab of model.tabs) {
      const ids = panesOf(tab.root);
      for (const id of ids) add([id], [paneOf(id)]);
      add(ids, ids.map(paneOf));
    }
    const every = model.tabs.flatMap((tab) => panesOf(tab.root));
    add(every, every.map(paneOf));
  }
  return result;
}

/** The desk's checkout: the project's main checkout, last committed ten minutes ago, with the desk's tabs. */
function deskCheckout(model: DeskModel, place: PracticePlace, scope: AgentScope, agents: AgentRow[], nowMs: number): Checkout {
  const id = checkoutId(place);
  const path = projectPath(place);
  const pane = (paneId: string) => {
    const row = model.panes.find((one) => one.id === paneId)!;
    return {
      id: paneId, herdr_label: null, terminal_title: row.agent ? null : row.title, cwd: path, status_code: "idle" as const,
      requires_close_confirmation: false, requires_close_status_check: false, identity_label: row.title,
    };
  };
  const tabs = model.tabs.map((tab, index) => ({
    id: tab.id, workspace_id: place.project, checkout_id: id, label: String(index + 1),
    sidebar: { number: index + 1, manual_name: tab.name, named_by_operator: tab.name !== null },
    empty: false, delegated: tabIsDelegated(model, tab), panes: panesOf(tab.root).map(pane),
  }));
  return {
    agent_scope: scope,
    id,
    workspace_id: place.project,
    label: place.branch,
    path,
    branch: place.branch,
    purpose: null,
    is_worktree: false,
    is_primary: true,
    exists: true,
    has_panes: model.panes.length > 0,
    agent_summary: {
      representative_pane_id: model.panes[0]?.id ?? null,
      needs_you: 0, done: 0, working: agents.length, seen: 0, unknown: 0,
      marks: { turn: 0, going: agents.length, done: 0, unread: 0 },
    },
    worktree: {
      path, branch: place.branch, head_sha: "0000000", last_commit_unix_seconds: nowMs / 1000 - 600, is_main: true, missing: false, dirty: false,
      changed_file_count: 0, merged: false, behind_upstream: null, pane_count: model.panes.length,
      deletion_gate: { blocked: null, warnings: [], pane_count: model.panes.length, can_delete_branch: true, branch_loss: null, discard: null },
    },
    pull_request: null,
    github: { failure_category: null, available: true, loading: false, stale: false, last_success_at_unix_ms: nowMs },
    tabs,
    active_tab_id: model.activeTab,
    // A tab holding only delegated children stays off the strip; the sidebar still reaches it.
    strip: tabs.filter((tab) => !tab.delegated).map((tab) => ({ id: `strip:${tab.id}`, kind: "herdr", source_id: tab.id, label: tab.sidebar.manual_name ?? `Tab ${tab.sidebar.number}`, preview: false })),
    next_tab_label: String(model.tabs.length + 1),
    agent_groups: { selected: model.selectedGroup, groups: model.groups, drafts: [] },
  };
}

function project(place: PracticePlace, checkouts: Checkout[], scope: AgentScope): Workspace {
  return {
    agent_scope: scope,
    label: place.label,
    path: projectPath(place),
    device_id: "local",
    registered: true,
    temporary: false,
    pinned: false,
    is_git: true,
    expanded: true,
    inactive_checkouts: { expanded: false, checkout_ids: [] },
    checkouts,
    agent_presence: { working: scope.has_working },
    id: place.project,
  };
}

const agentsOf = (model: DeskModel) => model.panes.flatMap((pane) => (pane.agent ? [pane.agent] : []));

/** A desk as the core lists it: its project, holding its main checkout. */
function deskProject({ place, model }: PracticeDesk, nowMs: number): Workspace {
  const agents = agentsOf(model);
  const scope = rootScope(agents.map((row) => ({ pane_id: row.pane_id, place })));
  return project(place, [deskCheckout(model, place, scope, agents, nowMs)], scope);
}

/**
 * The snapshot the core would send for `desks`, listed in that order, with
 * `front` the project in front, on this machine with the left sidebar on its
 * Projects list. Pane ids are unique across the desks, as Herdr's are.
 */
export function deskSnapshot(desks: readonly PracticeDesk[], front: string, nowMs: number): WorldSnapshot {
  const shownDesk = desks.find((desk) => desk.place.project === front);
  if (!shownDesk) throw new Error(`Practice project ${front} has no desk`);
  const model = shownDesk.model;
  const agents = desks.flatMap((desk) => agentsOf(desk.model));
  const scope = { ...rootScope(desks.flatMap(({ place, model: one }) => agentsOf(one).map((row) => ({ pane_id: row.pane_id, place })))), closes: closes(desks) };
  const workspaces = desks.map((desk) => deskProject(desk, nowMs));
  const frontProject = workspaces.find((row) => row.id === front)!;
  const main = frontProject.checkouts[0]!;
  const shown = model.groups.find((row) => row.id === model.selectedGroup)?.tabs ?? [];
  const groupFront = shown.includes(model.activeTab) ? model.activeTab : (shown[0] ?? null);
  const agentLayout: AgentLayout = {
    root: { area: { id: "a1", active: groupFront, displays: shown.map((id) => ({ id })) } },
    active_area: "a1", display_count: shown.length, limits: { areas: 6, depth: 3, displays: 64 }, canvases: {},
    group_id: model.selectedGroup, groups: model.groups, drafts: [], can_undo_group: false,
  };
  const focused = focusedPane(model);
  const rest: SnapshotRest = {
    navigator: {
      agent_scope: scope,
      devices: [{ agent_scope: scope, id: "local", label: "This Mac", kind: "local", state: "local", message: null, ssh_alias: null, agent_count: agents.length, test: null }],
      workspaces,
      agents,
      focused_workspace_id: front,
      focused_checkout_id: main.id,
      focused_device_id: "local",
      sleeping_sessions: [],
      inactive_projects: [],
      no_agent_projects: [],
    },
    ui_state: {
      left_sidebar_visible: true,
      collapsed_workspace_ids: [],
      session_collapsed_checkout_ids: [],
      workspace_registrations: workspaces.map((row) => ({ id: row.id, label: row.label, path: row.path, device_id: "local", pinned: false })),
      sidebar_lens: "projects",
      sidebar_width: 280,
      device_rail_visible: false,
    },
    focused: { pane_id: focused },
    status: { async_operations: desks.flatMap((desk) => desk.model.operations) },
    pane_layouts: desks.flatMap((desk) => desk.model.tabs.map((tab) => ({ workspace_id: desk.place.project, tab_id: tab.id, focused_pane_id: tab.focused, zoomed: tab.zoomed, root: tab.root }))),
    workspace_view: {
      device_id: "local", path: main.path, open: false, documents: false, tool: null, pinned: false,
      document_width: null, detail_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0, lane_called_side: "view",
      layout: { root: { area: { id: "v1", active: null, displays: [] } }, active_area: "v1", display_count: 0, limits: { areas: 4, depth: 3, displays: 32 } },
      agent_layout: agentLayout,
    },
    terminal: { headers: Object.fromEntries(agents.map((row) => [row.pane_id, { working: true, notice: null, band: null }])) },
  };
  return { rest, agents, focused, usage: null };
}

// --- Lineage and where a row stands -----------------------------------------------

/** `rest` with `change` made to every agent scope in it, and `workspace` to each project besides. */
function everyScope(rest: SnapshotRest, change: (scope: AgentScope) => AgentScope, workspace: (row: Workspace) => Partial<Workspace> = () => ({})): SnapshotRest {
  const navigator = rest.navigator!;
  return {
    ...rest,
    navigator: {
      ...navigator,
      agent_scope: navigator.agent_scope && change(navigator.agent_scope),
      devices: navigator.devices?.map((device) => ({ ...device, agent_scope: change(device.agent_scope) })),
      workspaces: navigator.workspaces?.map((row) => ({
        ...row,
        ...workspace(row),
        agent_scope: change(row.agent_scope),
        checkouts: (workspace(row).checkouts ?? row.checkouts).map((checkout) => ({ ...checkout, agent_scope: change(checkout.agent_scope) })),
      })),
    },
  };
}

/**
 * Every agent scope in `rest` with `children` drawn one level under `parent`
 * and no longer roots of their own, as the core draws a delegated child. The
 * kit lists every agent as a root.
 */
export function withLineage(rest: SnapshotRest, parent: string, children: readonly string[]): SnapshotRest {
  const nest = (tree: AgentTreeScope): AgentTreeScope => {
    const rows = tree.rows.filter((row) => !children.includes(row.pane_id)).flatMap((row) => (row.pane_id === parent ? [row, ...children.map((pane_id) => ({ pane_id, occurrence: 0, depth: 1 }))] : [row]));
    return { ...tree, rows, visible_rows: rows };
  };
  return everyScope(rest, (one) => ({
    ...one,
    roots: one.roots.filter((row) => !children.includes(row.pane_id)),
    children: { [parent]: [...children] },
    descendants: { [parent]: children.length },
    tree: nest(one.tree),
    sidebar_tree: nest(one.sidebar_tree),
    global_tree: nest(one.global_tree),
  }));
}

/**
 * Every agent scope in `rest` with `count` of its agents, one by default,
 * counted where their rows stand as the core counts them: Seen at rest, or
 * waiting on the operator in Needs You. The kit counts every agent as working.
 */
export function counted(rest: SnapshotRest, as: "seen" | "needs_you", count = 1): SnapshotRest {
  const asks = as === "needs_you" ? count : 0;
  const scope = (one: AgentScope): AgentScope => ({
    ...one,
    overview_needs_you: one.overview_needs_you + asks, has_working: one.groups.working > count,
    groups: { ...one.groups, [as]: one.groups[as] + count, working: one.groups.working - count },
    marks: { ...one.marks, turn: one.marks.turn + asks, going: one.marks.going - count },
    buckets: { ...one.buckets, [asks ? "turn" : "resting"]: one.buckets[asks ? "turn" : "resting"] + count, working: one.buckets.working - count },
    turns: { ...one.turns, question: one.turns.question + asks },
    tree: { ...one.tree, needs_you: one.tree.needs_you || asks > 0 },
    sidebar_tree: { ...one.sidebar_tree, needs_you: one.sidebar_tree.needs_you || asks > 0 },
    global_tree: { ...one.global_tree, needs_you: one.global_tree.needs_you || asks > 0 },
  });
  return everyScope(rest, scope, (workspace) => ({
    agent_presence: { working: workspace.agent_scope.groups.working > count },
    checkouts: workspace.checkouts.map((checkout) => ({
      ...checkout,
      agent_summary: checkout.agent_summary && {
        ...checkout.agent_summary, [as]: checkout.agent_summary[as] + count, working: checkout.agent_summary.working - count,
        marks: { ...checkout.agent_summary.marks, turn: checkout.agent_summary.marks.turn + asks, going: checkout.agent_summary.marks.going - count },
      },
    })),
  }));
}
