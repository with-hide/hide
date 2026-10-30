// A device's tasks as the Overview and the record read them (PRD
// overview-device-tasks D-05, B8-B10; PRD sidebar-task-lens D-20): which
// card an item or a pane belongs to, what a card's line says, its question
// and its pull requests, and the order the backlog draws a device's projects
// in. The core decides every card (`agent_scope.tasks`); this only words what
// a card already says.

import type { TaskCardScope } from "./agentScope";
import { frontDeviceId } from "./devices";
import { rowPulls } from "./prMark";
import type { BoardProject } from "./projectBoard";
import type { AgentRow, AgentStatusCode, PrState, RaiseVerb, SessionPull, SnapshotRest, Workspace } from "./snapshot";

/**
 * The device the Overview shows (PRD overview-device-tasks D-02): the one its
 * own switch picked, else the one a device Home row named, else the rail's
 * front device. A device removed since it was picked leaves the front one.
 */
export function overviewDeviceOf(rest: SnapshotRest | null, picked: string | null | undefined, named: string | null | undefined): string {
  const listed = (id: string | null | undefined): id is string => Boolean(id) && (rest?.navigator?.devices?.some((device) => device.id === id) ?? false);
  return listed(picked) ? picked : listed(named) ? named : frontDeviceId(rest);
}

/** Every agent row the snapshot holds, this machine's and each device's, by pane. */
export function everyAgent(rest: SnapshotRest | null, local: readonly AgentRow[]): Map<string, AgentRow> {
  const rows = new Map<string, AgentRow>();
  for (const status of rest?.status?.remote ?? []) for (const agent of status.session?.agents ?? []) rows.set(agent.pane_id, agent);
  for (const agent of local) rows.set(agent.pane_id, agent);
  return rows;
}

/** Whether a backlog key names a folder memo (`local:<path>#N`) rather than a GitHub issue. */
export function isMemoKey(key: string): boolean {
  return key.startsWith("local:");
}

/**
 * What an issue or memo is called on a chip (`#N`, `L-N`): the id its source
 * shows, else read from its key (`local:<path>#N`, `github:owner/repo#N`).
 */
export function itemToken(key: string, project: Pick<Workspace, "tasks">): string | null {
  const listed = project.tasks?.tasks.find((task) => task.key === key)?.id;
  if (listed) return listed;
  const number = key.slice(key.lastIndexOf("#") + 1);
  if (!/^\d+$/.test(number)) return null;
  return isMemoKey(key) ? `L-${number}` : key.startsWith("github:") ? `#${number}` : null;
}

/**
 * The device's projects in the backlog's order (B25): a project with a task
 * waiting on the operator first, then the most recently active, then by name;
 * `only` is the project chip (B5).
 */
export function projectOrder(projects: readonly BoardProject[], only: string | null): Workspace[] {
  return projects
    .map(({ workspace }) => workspace)
    .filter((workspace) => only === null || workspace.id === only)
    .sort((a, b) => {
      const tasksA = a.agent_scope.tasks;
      const tasksB = b.agent_scope.tasks;
      return Number((tasksB?.needs_you ?? 0) > 0) - Number((tasksA?.needs_you ?? 0) > 0) || (tasksB?.recency ?? "").localeCompare(tasksA?.recency ?? "") || a.label.localeCompare(b.label);
    });
}

/** The task's pull requests (B10): its root's list, which holds every member's, live or ended, once each. */
export function cardPulls(card: Pick<TaskCardScope, "root">, rows: ReadonlyMap<string, AgentRow>): SessionPull[] {
  const root = rows.get(card.root);
  return root ? rowPulls(root) : [];
}

/** The issue and memo chips a task wears (`#N`/`L-N`, B8): its checkouts' issue links, once each. */
function cardItems(card: Pick<TaskCardScope, "checkouts">, project: Workspace): { key: string; token: string }[] {
  const items: { key: string; token: string }[] = [];
  for (const id of card.checkouts) {
    const key = project.checkouts.find((row) => row.id === id)?.task_key;
    const token = key ? itemToken(key, project) : null;
    if (key && token && !items.some((item) => item.key === key)) items.push({ key, token });
  }
  return items;
}

/** The card a task key is on, for the ways in that name an issue (D-31). */
export function cardOfItem(projects: readonly BoardProject[], key: string): { project: Workspace; card: TaskCardScope } | null {
  for (const { workspace } of projects) {
    for (const card of workspace.agent_scope.tasks?.cards ?? []) {
      if (cardItems(card, workspace).some((item) => item.key === key)) return { project: workspace, card };
    }
  }
  return null;
}

/** The card a pane belongs to, root or member, on any of the projects (B59). */
export function cardOfPane(projects: readonly BoardProject[], pane: string): { project: Workspace; card: TaskCardScope } | null {
  for (const { workspace } of projects) {
    for (const card of workspace.agent_scope.tasks?.cards ?? []) if (card.members.includes(pane)) return { project: workspace, card };
  }
  return null;
}

/**
 * What a card's third line says (B9), as a kind the card words: the reason it
 * waits on the operator or is blocked, that it merged, its open PR's state,
 * that it is working, that it finished unread, or that it rests.
 */
export type CardLine =
  | { kind: "needs_you"; verb: RaiseVerb | null; what: string | null; agent: string }
  | { kind: "blocked"; what: string | null }
  | { kind: "merged"; cleanup: boolean }
  | { kind: "pr"; number: number; state: PrState }
  | { kind: "working"; agents: number }
  | { kind: "done"; status: AgentStatusCode | null }
  | { kind: "resting" };

export function cardLine(card: TaskCardScope, rows: ReadonlyMap<string, AgentRow>): CardLine {
  const root = rows.get(card.root);
  if (card.state === "needs_you") return { kind: "needs_you", verb: root?.state.ask?.verb ?? null, what: root?.state.ask?.what ?? null, agent: root?.agent_kind ?? "" };
  if (card.state === "blocked") return { kind: "blocked", what: root?.request?.line ?? root?.detail ?? null };
  if (card.stage === 4) return { kind: "merged", cleanup: card.state === "cleanup" };
  const open = cardPulls(card, rows).find((pull) => pull.state !== "merged" && pull.state !== "closed");
  if (open) return { kind: "pr", number: open.number, state: open.state };
  if (card.state === "working") return { kind: "working", agents: card.members.filter((pane) => rows.get(pane)?.activity === "working").length };
  if (card.state === "done") return { kind: "done", status: root?.status_code ?? null };
  return { kind: "resting" };
}

/** The question a task asks in one line, when its root is asking (B8). */
export function cardQuestion(card: TaskCardScope, rows: ReadonlyMap<string, AgentRow>): string | null {
  if (card.state !== "needs_you") return null;
  const root = rows.get(card.root);
  const text = root?.user_turn?.content?.text ?? root?.state.ask?.what ?? null;
  return text ? text.split(/\r?\n/).find((line) => line.trim() !== "")?.trim() ?? null : null;
}
