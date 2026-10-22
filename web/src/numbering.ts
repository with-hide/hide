import { deviceScope } from "./agentScope";
// What ⌥n and ⌘n select, and the number each agent row and device tile
// carries while a hold reveals it (PRD electron-digit-shortcuts-hints D-02).
// The number is the screen order at that moment, first to ninth: the rail's
// devices top to bottom, the sidebar list's drawn rows top to bottom, the
// 프로젝트 tree's roots or, while the `프로젝트 | 태스크` switch shows the 태스크
// list, its task rows (PRD sidebar-task-lens). A tenth item has no number,
// and a number with nothing at it selects nothing.

import { asksOperator, type TreeRow } from "./agentRow";
import { frontDeviceId, railDeviceIds } from "./devices";
import { deviceListedAgents } from "./navigation";
import { agentsByPane, allAgents as everyPaneAgent } from "./paneAgents";
import { projectRows, activeCheckouts, inactiveCheckouts, folderCheckout, projectExpanded } from "./projects";
import { checkoutAgentRows } from "./projectBoard";
import { sidebarNamedGroups, sidebarTabGroups } from "./sidebarTabs";
import { groupFoldKey, useGroupFolds } from "./groupTabs";
import { contextWorkspaces, contextAgents } from "./remote";
import { DIGITS, type Digit } from "./shortcuts";
import { catalogWorkspaces, type AgentRow, type Checkout, type Device, type SnapshotRest } from "./snapshot";
import { drawnTasks, taskList, taskSections } from "./taskList";

/** The page-local folds the two lists' drawn orders depend on (`ui.ts`). */
export type SidebarFolds = { raisedOpen: readonly string[]; taskFoldsOpen: readonly string[] };

/** The pane id at each number, the first nine of `panes` in their order, a pane counted once. */
export function numberedPanes(panes: readonly string[]): Map<Digit, string> {
  const numbered = new Map<Digit, string>();
  const seen = new Set<string>();
  panes.filter((pane) => !seen.has(pane) && !!seen.add(pane)).slice(0, DIGITS.length).forEach((pane, index) => numbered.set(DIGITS[index]!, pane));
  return numbered;
}

/**
 * The device id at each number ⌘n selects: the rail's tiles top to bottom,
 * the window's own machine first, numbered the same whether the rail shows
 * or not. A tile drawn before the core named its machine has no id to
 * select, so it holds no number.
 */
export function numberedDevices(devices: readonly Device[] | undefined): Map<Digit, string> {
  return numberedPanes(railDeviceIds(devices).filter((id) => id !== ""));
}

/** The pane id at each number of the Agents list's drawn rows, in number order. */
export function numberedAgents(rows: readonly TreeRow[]): Map<Digit, string> {
  return numberedPanes(rows.map((row) => row.agent.pane_id));
}

/** The number an item holds, by its id, or null past the ninth. */
export function numberOf(numbered: Map<Digit, string>, id: string): Digit | null {
  for (const [digit, held] of numbered) if (held === id) return digit;
  return null;
}

/** The Agents list's rows in draw order, the list ⌥n numbers whichever list is on screen (B2): the front device's. */
function sidebarAgentOrder(state: { rest: SnapshotRest | null; agents: AgentRow[] }, raisedOpen: readonly string[], groupClosed: ReadonlySet<string>): { row: TreeRow; place: string | null }[] {
  const { rest } = state;
  const scope = deviceScope(rest, frontDeviceId(rest));
  const agents = contextAgents(rest, state.agents);
  const workspaces = contextWorkspaces(rest);
  const listed = deviceListedAgents(rest?.status?.remote, rest?.navigator?.devices, state.agents, frontDeviceId(rest));
  const projects = projectRows(workspaces, rest?.navigator ?? {}, listed, scope, raisedOpen);
  const rows: { row: TreeRow; place: string | null }[] = [];
  const append = (agent: AgentRow, place: string | null = null) => rows.push({ row: { agent, device: null, depth: 0, descendants: (agent.lineage_child_pane_ids ?? []).length }, place });
  for (const row of projects) if (row.kind === "raised") for (const item of [...row.agents, ...(row.expanded ? row.more : [])]) append(item.agent);
  const collapsed = rest?.ui_state?.session_collapsed_checkout_ids ?? [];
  const checkout = (workspace: import("./snapshot").Workspace, value: Checkout) => {
    const grouped = sidebarTabGroups(value, workspace.device_id, checkoutAgentRows(workspace, agents, "sidebar").get(value.id) ?? [], agents);
    const ordered = value.agent_groups && !collapsed.includes(value.id) ? sidebarNamedGroups(value, grouped).filter(({ group }) => !groupClosed.has(groupFoldKey(workspace.device_id, value.id, group.id))).flatMap(({ segments }) => segments) : grouped;
    for (const row of ordered.flatMap((group) => group.rows)) {
      // Digits number the roots; an opened root's children and grandchildren take none.
      if (row.depth === 0 && (!collapsed.includes(value.id) || asksOperator(row.agent))) append(row.agent, value.id);
    }
  };
  for (const row of projects) {
    if (row.kind !== "workspace") continue;
    const workspace = row.workspace;
    const folder = folderCheckout(workspace);
    if (folder) {
      // A folder has no project fold of its own, only the sleeping one (D-03).
      const asleep = workspace.agent_presence?.asleep;
      if (!workspace.session_folds?.cleanup.includes(folder.id) && (!asleep || asleep.expanded)) checkout(workspace, folder);
      continue;
    }
    if (!projectExpanded(workspace)) continue;
    // A checkout Hide AI made with no operator agent in it has no row, so the numbers follow the drawing without a rule of their own.
    for (const value of activeCheckouts(workspace)) checkout(workspace, value);
    if (workspace.session_folds?.empty_open) for (const value of workspace.checkouts.filter((value) => workspace.session_folds!.empty.includes(value.id))) checkout(workspace, value);
    if (workspace.inactive_checkouts.expanded) for (const value of inactiveCheckouts(workspace)) checkout(workspace, value);
  }
  for (const workspace of workspaces) if (workspace.session_folds?.cleanup_open) for (const value of workspace.checkouts.filter((value) => workspace.session_folds!.cleanup.includes(value.id))) checkout(workspace, value);
  return rows;
}

/** Sidebar order is the actual drawn order, including each open fold. */
export function agentListOrder(state: { rest: SnapshotRest | null; agents: AgentRow[] }, raisedOpen: readonly string[] = [], groupClosed: ReadonlySet<string> = useGroupFolds.getState().closed): TreeRow[] {
  return sidebarAgentOrder(state, raisedOpen, groupClosed).map(({ row }) => row);
}

/** A physical pane gets a keycap only at its first visible appearance. */
export function sidebarAgentNumbers(state: { rest: SnapshotRest | null; agents: AgentRow[] }, raisedOpen: readonly string[], groupClosed: ReadonlySet<string> = useGroupFolds.getState().closed) {
  const entries = sidebarAgentOrder(state, raisedOpen, groupClosed);
  const numbers = numberedAgents(entries.map(({ row }) => row));
  const first = new Map<string, string | null>();
  for (const { row, place } of entries) if (!first.has(row.agent.pane_id)) first.set(row.agent.pane_id, place);
  return (pane: string, place: string | null) => first.has(pane) && first.get(pane) === place ? numberOf(numbers, pane) : null;
}

/** Which sidebar list ⌥n counts: the one the `프로젝트 | 태스크` switch shows, whether or not the sidebar is open. */
export function numberedList(rest: SnapshotRest | null): "projects" | "tasks" {
  return rest?.ui_state?.sidebar_lens === "tasks" ? "tasks" : "projects";
}

/**
 * The 태스크 list's task rows top to bottom as the page draws them (PRD
 * sidebar-task-lens B4-B6): each group's rows, the rows of the 쉬는 중 and
 * 정리할 것 folds the page opened, a task's children never among them.
 */
export function taskListOrder(state: { rest: SnapshotRest | null; agents: AgentRow[] }, taskFoldsOpen: readonly string[]): string[] {
  const { rest } = state;
  const view = rest?.ui_state?.task_view ?? "status";
  const list = taskList(deviceScope(rest, frontDeviceId(rest))?.tasks, agentsByPane(everyPaneAgent(state.agents, rest?.status?.remote)), catalogWorkspaces(rest), view);
  return drawnTasks(taskSections(list, view, new Set(taskFoldsOpen))).map(({ agent }) => agent.pane_id);
}

/** The pane at each ⌥n now: the 태스크 list's rows while the switch shows that list, else the 프로젝트 list's roots. */
export function sidebarNumbers(state: { rest: SnapshotRest | null; agents: AgentRow[] }, folds: SidebarFolds): Map<Digit, string> {
  return numberedList(state.rest) === "tasks" ? numberedPanes(taskListOrder(state, folds.taskFoldsOpen)) : numberedAgents(agentListOrder(state, folds.raisedOpen));
}

/** The number ⌥n selects an agent by now, or null past the ninth row. */
export function agentNumber(state: { rest: SnapshotRest | null; agents: AgentRow[] }, paneId: string, folds: SidebarFolds = { raisedOpen: [], taskFoldsOpen: [] }): Digit | null {
  return numberOf(sidebarNumbers(state, folds), paneId);
}
