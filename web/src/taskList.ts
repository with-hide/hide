// The sidebar's 태스크 list as data (PRD sidebar-task-lens B4-B9): the rail's
// device's tasks in the core's order (`agent_scope.tasks` of the device),
// cut into the groups the list draws. The core decides each task's state and
// order for open tasks; this places a task under its state or its project,
// orders resting rows by the request age they display, and folds them.

import type { ProjectTasksScope, TaskCardScope, TaskState } from "./agentScope";
import { archivedProject } from "./archive";
import type { AgentRow, ArchivedSession, TaskView, Workspace } from "./snapshot";

/**
 * The open groups of the 상태별 view and the bands of a 프로젝트별 group, in
 * the order they are drawn (status-marks D-22, B32): 내 차례 (a question, an
 * approval or a block), 읽을 것, whose tasks want the operator's look, above
 * 일하는 중, whose agents work, and 대기 last, whose tasks wait on proven work.
 */
export type TaskGroupState = "needs_you" | "done" | "working" | "waiting";
export const TASK_GROUPS: readonly TaskGroupState[] = ["needs_you", "done", "working", "waiting"];

export type TaskEntry = { card: TaskCardScope; agent: AgentRow; project: Workspace | null };

/** An archived root under `보관됨` (PRD session-archive B14, B15). */
export type ArchivedEntry = { row: ArchivedSession; project: Workspace | null };

export type TaskGroup = {
  /** `state:<state>` or `project:<id>`, also the folds' key. */
  key: string;
  state: TaskGroupState | null;
  project: Workspace | null;
  /** The open rows, band by band, each band in the core's order (B9). */
  entries: TaskEntry[];
  /** The open rows cut by state in `TASK_GROUPS` order; a 상태별 group has one band, a 프로젝트별 group labels each (B32). */
  bands: { state: TaskGroupState; entries: TaskEntry[] }[];
  /** Per state, the open rows a project's head counts (B6). */
  counts: Partial<Record<TaskGroupState, number>>;
  resting: TaskEntry[];
  cleanup: TaskEntry[];
  /** The project's archived roots, newest archived first, folded below 정리할 것 (프로젝트별 only). */
  archived: ArchivedEntry[];
};

export type TaskList = {
  /** Open tasks, the folded ones not counted. */
  total: number;
  groups: TaskGroup[];
  /** The 상태별 view's folds, drawn once under every group; `보관됨` the very last. */
  resting: TaskEntry[];
  cleanup: TaskEntry[];
  archived: ArchivedEntry[];
};

/** Which open group a state stands in: a block is the operator's turn (B32). */
export function groupOf(state: TaskState): TaskGroupState | "resting" | "cleanup" {
  return state === "blocked" ? "needs_you" : state;
}

/**
 * The list for one view. A card whose root row the snapshot no longer holds
 * is left out until it does, rather than drawn without a title.
 */
export function taskList(tasks: ProjectTasksScope | null | undefined, rows: ReadonlyMap<string, AgentRow>, projects: readonly Workspace[], view: TaskView, archivedRows: readonly ArchivedSession[] = []): TaskList {
  const byCheckout = new Map(projects.flatMap((project) => project.checkouts.map((checkout) => [checkout.id, project] as const)));
  const entries: TaskEntry[] = [];
  for (const card of tasks?.cards ?? []) {
    const agent = rows.get(card.root);
    if (!agent) continue;
    entries.push({ card, agent, project: card.checkouts.map((id) => byCheckout.get(id)).find((row) => row !== undefined) ?? null });
  }
  const open = entries.filter(({ card }) => card.state !== "resting" && card.state !== "cleanup");
  const resting = entries.filter(({ card }) => card.state === "resting")
    .sort((a, b) => {
      const at = a.agent.last_prompt_at_unix_ms;
      const bt = b.agent.last_prompt_at_unix_ms;
      if (at == null) return bt == null ? 0 : 1;
      if (bt == null) return -1;
      return bt - at;
    });
  const cleanup = entries.filter(({ card }) => card.state === "cleanup");
  const archived = archivedRows.map((row) => ({ row, project: archivedProject(row, projects) }));
  if (view === "status") {
    const groups = TASK_GROUPS.map((state) => {
      const entries = open.filter(({ card }) => groupOf(card.state) === state);
      return { key: `state:${state}`, state, project: null, entries, bands: [{ state, entries }], counts: {}, resting: [], cleanup: [], archived: [] };
    }).filter((group) => group.entries.length > 0);
    return { total: open.length, groups, resting, cleanup, archived };
  }
  // One group per project, the projects that ask first, then in the order their first task stands (B6).
  const order: (Workspace | null)[] = [];
  for (const { project } of entries) if (!order.includes(project)) order.push(project);
  // A project whose sessions are all archived still stands, after the ones with work.
  for (const { project } of archived) if (!order.includes(project)) order.push(project);
  const asks = (project: Workspace | null) => open.some((entry) => entry.project === project && groupOf(entry.card.state) === "needs_you");
  const sorted = [...order.filter(asks), ...order.filter((project) => !asks(project))];
  const groups = sorted.map((project) => {
    const mine = (list: TaskEntry[]) => list.filter((entry) => entry.project === project);
    const shown = mine(open);
    const bands = TASK_GROUPS.map((state) => ({ state, entries: shown.filter(({ card }) => groupOf(card.state) === state) })).filter((band) => band.entries.length > 0);
    const counts: Partial<Record<TaskGroupState, number>> = Object.fromEntries(bands.map((band) => [band.state, band.entries.length]));
    return { key: `project:${project?.id ?? ""}`, state: null, project, entries: bands.flatMap((band) => band.entries), bands, counts, resting: mine(resting), cleanup: mine(cleanup), archived: archived.filter((entry) => entry.project === project) };
  });
  return { total: open.length, groups, resting: [], cleanup: [], archived: [] };
}

/** A fold's key in the page's open set (B5): `rest:`, `cleanup:` or `archived:` and the group it belongs to, the empty group being the 상태별 view's folds under every group. */
export function taskFoldKey(kind: "rest" | "cleanup" | "archived", group: string): string {
  return `${kind}:${group}`;
}

export type TaskSection = {
  group: TaskGroup;
  /** The group's open rows, then the rows of each of its folds the page opened (프로젝트별 only). */
  resting: TaskEntry[];
  cleanup: TaskEntry[];
  archived: ArchivedEntry[];
};

export type TaskSections = {
  sections: TaskSection[];
  /** The 상태별 view's folds under every group, their rows only while open. */
  resting: TaskEntry[];
  cleanup: TaskEntry[];
  archived: ArchivedEntry[];
};

/**
 * What the list draws for one page: every group's rows and the rows of the
 * folds in `open`. The list draws from this, and the ⌥n numbers read it, so
 * a number always names a row the operator can see.
 */
export function taskSections(list: TaskList, view: TaskView, open: ReadonlySet<string>): TaskSections {
  const folded = <T,>(entries: T[], key: string): T[] => (open.has(key) ? entries : []);
  const sections = list.groups.map((group) => ({
    group,
    resting: view === "project" ? folded(group.resting, taskFoldKey("rest", group.key)) : [],
    cleanup: view === "project" ? folded(group.cleanup, taskFoldKey("cleanup", group.key)) : [],
    archived: view === "project" ? folded(group.archived, taskFoldKey("archived", group.key)) : [],
  }));
  return {
    sections,
    resting: view === "status" ? folded(list.resting, taskFoldKey("rest", "")) : [],
    cleanup: view === "status" ? folded(list.cleanup, taskFoldKey("cleanup", "")) : [],
    archived: view === "status" ? folded(list.archived, taskFoldKey("archived", "")) : [],
  };
}

/** The task rows top to bottom as the page draws them, a task's children not among them: what ⌥1 to ⌥9 count. */
export function drawnTasks(drawn: TaskSections): TaskEntry[] {
  return [...drawn.sections.flatMap(({ group, resting, cleanup }) => [...group.entries, ...resting, ...cleanup]), ...drawn.resting, ...drawn.cleanup];
}
