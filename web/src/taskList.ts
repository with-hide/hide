// The sidebar's 태스크 list as data (PRD sidebar-task-lens B4-B9): the rail's
// device's tasks in the core's order (`agent_scope.tasks` of the device),
// cut into the groups the list draws. The core decides each task's state and
// order for open tasks; this places a task under its state or its project,
// orders resting rows by the request age they display, and folds them.

import type { ProjectTasksScope, TaskCardScope, TaskState } from "./agentScope";
import { archivedProject } from "./archive";
import type { MissionAsk } from "./missions";
import type { AgentRow, ArchivedSession, Device, TaskView, Workspace } from "./snapshot";

/**
 * The open groups of the 상태별 view and the bands of a 프로젝트별 group, in
 * the order they are drawn (status-marks D-22, B32): 내 차례 (a question, an
 * approval or a block), 읽을 것, whose tasks want the operator's look, 다시 볼 것
 * (PRD revisit-group D-01), a task the operator starred (a read ask stays in
 * 내 차례, needs-you-cleanup D-04),
 * above 작업 중, whose tasks are in progress: the ones whose agent works first,
 * then the ones that wait on proven work (CI, children, a reply), since
 * neither asks anything of the operator.
 */
export type TaskGroupState = "needs_you" | "done" | "revisit" | "working";
export const TASK_GROUPS: readonly TaskGroupState[] = ["needs_you", "done", "revisit", "working"];

/** `device` and `deviceId`: the name and id of another device than the one in front, whose 내 차례 task this is (`elsewhereTasks`); null for the device's own. */
export type TaskEntry = { card: TaskCardScope; agent: AgentRow; project: Workspace | null; device: string | null; deviceId: string | null };

/** Another device's task in 내 차례, with that device's name for its row's chip. */
export type ElsewhereTask = { card: TaskCardScope; device: string; deviceId: string };

/** An archived root under `보관됨` (PRD session-archive B14, B15). */
export type ArchivedEntry = { row: ArchivedSession; project: Workspace | null };

export type TaskGroup = {
  /** `state:<state>` or `project:<id>`, also the folds' key. */
  key: string;
  state: TaskGroupState | null;
  project: Workspace | null;
  /** The open rows, band by band, each band in the core's order (B9). */
  entries: TaskEntry[];
  /**
   * The open rows cut by state in `TASK_GROUPS` order; a 상태별 group has one band, a 프로젝트별 group labels each (B32).
   * The 내 차례 band's `missions` are the missions that are their own Needs You item, drawn among its rows (`rows`).
   */
  bands: TaskBand[];
  /** Per state, the open rows and mission rows a project's head counts (B6). */
  counts: Partial<Record<TaskGroupState, number>>;
  resting: TaskEntry[];
  cleanup: TaskEntry[];
  /** The project's archived roots, newest archived first, folded below 정리할 것 (프로젝트별 only). */
  archived: ArchivedEntry[];
};

/** One row of a band in the order drawn: a task, or a mission that is its own Needs You item. */
export type BandRow = { kind: "task"; entry: TaskEntry } | { kind: "mission"; ask: MissionAsk };

export type TaskBand = { state: TaskGroupState; entries: TaskEntry[]; missions: MissionAsk[]; rows: BandRow[] };

/** The rows a band or a group draws: its tasks and its mission rows. */
export function rowCount(bands: readonly TaskBand[]): number {
  return bands.reduce((sum, band) => sum + band.entries.length + band.missions.length, 0);
}

export type TaskList = {
  /** The device's own open tasks, the folded ones and other devices' not counted. */
  total: number;
  groups: TaskGroup[];
  /** The 상태별 view's folds, drawn once under every group; `보관됨` the very last. */
  resting: TaskEntry[];
  cleanup: TaskEntry[];
  archived: ArchivedEntry[];
};

/** Which open group a state stands in: a block is the operator's turn (B32), and a wait is work in progress. */
export function groupOf(state: TaskState): TaskGroupState | "resting" | "cleanup" {
  return state === "blocked" ? "needs_you" : state === "waiting" ? "working" : state;
}

/** A group's open rows in the core's order, a working agent's above a waiting one's. */
function inGroup(open: readonly TaskEntry[], state: TaskGroupState): TaskEntry[] {
  const entries = open.filter(({ card }) => groupOf(card.state) === state);
  return state === "working" ? [...entries.filter(({ card }) => card.state !== "waiting"), ...entries.filter(({ card }) => card.state === "waiting")] : entries;
}

/** A band of `state`, holding `missions` only in 내 차례. */
function band(state: TaskGroupState, entries: TaskEntry[], missions: readonly MissionAsk[], deviceId: string): TaskBand {
  const held = state === "needs_you" ? [...missions] : [];
  return { state, entries, missions: held, rows: bandRows(entries, held, deviceId) };
}

/**
 * A band's rows in the order drawn (operator decision 2026-11-08): device by
 * device, `deviceId`'s first, as its tasks already stand; inside a device a
 * mission stands among the unread asks by how long its node has waited, and
 * before every read ask and block. The tasks keep the core's order (unread
 * asks before read ones, each longest waiting first) and the missions the
 * scope's (oldest first).
 */
function bandRows(entries: readonly TaskEntry[], missions: readonly MissionAsk[], deviceId: string): BandRow[] {
  const deviceOf = (entry: TaskEntry) => entry.deviceId ?? deviceId;
  const devices = [...new Set([deviceId, ...entries.map(deviceOf), ...missions.map((ask) => ask.mission.device_id)])];
  return devices.flatMap((device) => {
    const waiting = missions.filter((ask) => ask.mission.device_id === device);
    const rows: BandRow[] = [];
    let next = 0;
    for (const entry of entries.filter((entry) => deviceOf(entry) === device)) {
      // An unread ask waits since its request, the core's own key; a read ask or a block lets every mission stand first.
      const unread = entry.card.state === "needs_you" && !entry.card.read;
      while (next < waiting.length && (!unread || waiting[next]!.attention.since_unix_ms < (entry.agent.state.request_since ?? -Infinity))) rows.push({ kind: "mission", ask: waiting[next++]! });
      rows.push({ kind: "task", entry });
    }
    for (const ask of waiting.slice(next)) rows.push({ kind: "mission", ask });
    return rows;
  });
}

/**
 * The list for one view. A card whose root row the snapshot no longer holds
 * is left out until it does, rather than drawn without a title. `missions`
 * are the device's missions that are their own Needs You item
 * (`useMissionAsks`), counted and drawn in 내 차례 with the tasks, under
 * their project in 프로젝트별. `elsewhere` are other devices' 내 차례 tasks,
 * after the device's own in 내 차례 and under their own projects in 프로젝트별
 * (operator decision 2026-11-08); the total leaves them out.
 */
export function taskList(tasks: ProjectTasksScope | null | undefined, deviceId: string, rows: ReadonlyMap<string, AgentRow>, projects: readonly Workspace[], view: TaskView, archivedRows: readonly ArchivedSession[] = [], missions: readonly MissionAsk[] = [], elsewhere: readonly ElsewhereTask[] = []): TaskList {
  const byCheckout = new Map(projects.flatMap((project) => project.checkouts.map((checkout) => [checkout.id, project] as const)));
  const entries: TaskEntry[] = [];
  const cards = [...(tasks?.cards ?? []).map((card) => ({ card, device: null, deviceId: null })), ...elsewhere];
  for (const { card, device, deviceId: from } of cards) {
    const agent = rows.get(card.root);
    if (!agent) continue;
    entries.push({ card, agent, device, deviceId: from, project: card.checkouts.map((id) => byCheckout.get(id)).find((row) => row !== undefined) ?? null });
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
      const entries = inGroup(open, state);
      return { key: `state:${state}`, state, project: null, entries, bands: [band(state, entries, missions, deviceId)], counts: {}, resting: [], cleanup: [], archived: [] };
    }).filter((group) => rowCount(group.bands) > 0);
    return { total: open.filter((entry) => entry.device === null).length, groups, resting, cleanup, archived };
  }
  // One group per project, the projects that ask first, then in the order their first task stands (B6).
  const order: (Workspace | null)[] = [];
  const missionProject = (ask: MissionAsk) => projects.find((project) => project.id === ask.mission.workspace_id) ?? null;
  for (const { project } of entries) if (!order.includes(project)) order.push(project);
  for (const ask of missions) if (!order.includes(missionProject(ask))) order.push(missionProject(ask));
  // A project whose sessions are all archived still stands, after the ones with work.
  for (const { project } of archived) if (!order.includes(project)) order.push(project);
  const asks = (project: Workspace | null) => missions.some((ask) => missionProject(ask) === project) || open.some((entry) => entry.project === project && groupOf(entry.card.state) === "needs_you");
  const sorted = [...order.filter(asks), ...order.filter((project) => !asks(project))];
  const groups = sorted.map((project) => {
    const mine = (list: TaskEntry[]) => list.filter((entry) => entry.project === project);
    const shown = mine(open);
    const asked = missions.filter((ask) => missionProject(ask) === project);
    const bands = TASK_GROUPS.map((state) => band(state, inGroup(shown, state), asked, deviceId)).filter((band) => rowCount([band]) > 0);
    const counts: Partial<Record<TaskGroupState, number>> = Object.fromEntries(bands.map((band) => [band.state, rowCount([band])]));
    return { key: `project:${project?.id ?? ""}`, state: null, project, entries: bands.flatMap((band) => band.entries), bands, counts, resting: mine(resting), cleanup: mine(cleanup), archived: archived.filter((entry) => entry.project === project) };
  });
  return { total: open.filter((entry) => entry.device === null).length, groups, resting: [], cleanup: [], archived: [] };
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

/**
 * The device's own task rows top to bottom as the page draws them, a task's
 * children and other devices' 내 차례 rows not among them: what ⌥1 to ⌥9 count.
 */
export function drawnTasks(drawn: TaskSections): TaskEntry[] {
  return [...drawn.sections.flatMap(({ group, resting, cleanup }) => [...group.entries, ...resting, ...cleanup]), ...drawn.resting, ...drawn.cleanup].filter((entry) => entry.device === null);
}

/**
 * The 내 차례 tasks of every connected device but `frontId`, in the devices'
 * order and each device's own, with the device's name (operator decision
 * 2026-11-08).
 */
export function elsewhereTasks(devices: readonly Device[] | undefined, frontId: string, connected: (deviceId: string) => boolean, name: (deviceId: string) => string): ElsewhereTask[] {
  return (devices ?? []).flatMap((device) => device.id === frontId || !connected(device.id) ? [] : (device.agent_scope.tasks?.cards ?? []).filter((card) => groupOf(card.state) === "needs_you").map((card) => ({ card, device: name(device.id), deviceId: device.id })));
}
