import { summariesEnabled } from "./hideAi";
import { agentsByPane, allAgents } from "./paneAgents";
import type { BoardProject } from "./projectBoard";
import { localDeviceId, type AgentRow, type Checkout, type LinkSummaries, type Workspace, type MissedWork, type PrState, type SessionRecord, type SessionWorkSummary, type SnapshotRest, type WorkOutput, type WorkProgress, type WorkRequest, type WorkSession, type WorkTask } from "./snapshot";

export const workKey = (session: WorkSession) => `${session.device_id}\0${session.agent}\0${session.session_id}`;
export function sessionOfAgent(agent: AgentRow, rest: SnapshotRest | null): WorkSession | null {
  return sessionOfAgentOn(agent, localDeviceId(rest));
}
/** The same, given this machine's id, for a caller that should not read the whole snapshot. */
export function sessionOfAgentOn(agent: AgentRow, localId: string): WorkSession | null {
  return agent.session_id ? { device_id: agent.device_id ?? localId, agent: agent.agent_kind, session_id: agent.session_id } : null;
}
/**
 * The live pane's tasks and children as the core published them for its
 * session; null for a pane with no task, a shell, or with Agent summaries
 * off, which keep no tasks (PRD task-split-checklist B9, B15). The row is
 * the snapshot's own, so a store selector stays stable.
 */
export function paneProgress(state: { agents: AgentRow[]; rest: SnapshotRest | null; linkSummaries: LinkSummaries | null }, paneId: string): WorkProgress | null {
  if (!summariesEnabled(state.rest?.status?.background_ai)) return null;
  const agent = agentsByPane(allAgents(state.agents, state.rest?.status?.remote)).get(paneId);
  const session = agent ? sessionOfAgentOn(agent, localDeviceId(state.rest)) : null;
  if (!session) return null;
  const key = workKey(session);
  return state.linkSummaries?.progress?.find((row) => workKey(row.session) === key) ?? null;
}
export function recordTitle(record: Pick<SessionRecord, "title" | "manual_title" | "native_title" | "fallback_title">, summaries: boolean): string {
  return record.manual_title ?? (summaries ? record.title : record.native_title ?? record.fallback_title);
}
export function recordTree(record: SessionRecord): SessionRecord[] {
  return [record, ...record.children.flatMap(recordTree)];
}
/**
 * The group a task belongs to (PRD task-split-checklist D-03): the tasks one
 * request made share `task-<offset>`, the first carrying it and the rest
 * `task-<offset>.<n>`. They share every END and every pull request.
 */
export function taskGroup(id: string): string {
  const dot = id.indexOf(".");
  return dot < 0 ? id : id.slice(0, dot);
}
/** The pull requests assigned to any task of `task`'s group. */
export function groupPrs(task: WorkTask, outputs: readonly WorkOutput[]): WorkOutput[] {
  const group = taskGroup(task.id);
  return outputs.filter((output) => output.kind === "pr" && output.task_id !== null && taskGroup(output.task_id) === group);
}
export function taskState(task: WorkTask, outputs: readonly WorkOutput[]) {
  const prs = groupPrs(task, outputs);
  if (prs.some((output) => output.state === "ci_failed")) return "ci_failed";
  if (prs.some((output) => ["open", "review"].includes(output.state))) return "review";
  if (task.done) return "done";
  // The newest turn went unjudged: its END is an earlier turn's, so no word (PRD task-continuation D-09).
  if (task.unjudged) return "unjudged";
  return task.end === "done" ? "unfinished" : task.end === "waiting" ? "working" : task.end;
}
const OUTPUT_ORDER: Record<string, number> = { ci_failed: 0, review: 1, open: 2, merged: 3, closed: 4 };
export function orderedOutputs(outputs: readonly WorkOutput[]): WorkOutput[] {
  return [...outputs].sort((a, b) => (OUTPUT_ORDER[a.state] ?? 5) - (OUTPUT_ORDER[b.state] ?? 5) || b.created_at_unix_ms - a.created_at_unix_ms);
}
/**
 * Every open missed item of the sessions given, of a live session as much as
 * of an ended one: a session that works again keeps what it missed in the
 * Overview backlog's group, and the newest comes first.
 */
export function openMissedWork(work: readonly SessionWorkSummary[]): { owner: SessionWorkSummary; item: MissedWork }[] {
  return work
    .flatMap((owner) => owner.missed.filter((item) => item.status === "open").map((item) => ({ owner, item })))
    .sort((a, b) => b.item.at_unix_ms - a.item.at_unix_ms);
}

/** The outputs the record's own session made. A task id names a turn offset, so a descendant's output can carry the same id as a task here and must not be read against it. */
function ownOutputs(record: SessionRecord): WorkOutput[] {
  const own = workKey(record.session);
  return record.outputs.filter((output) => workKey(output.source) === own);
}
/** The outputs a child and its descendants made, as the parent's record lists them and as the child's own does, each once (B15). */
export function childOutputs(parent: SessionRecord, child: SessionRecord): WorkOutput[] {
  const below = new Set(recordTree(child).map((record) => workKey(record.session)));
  const seen = new Set<string>();
  const rows: WorkOutput[] = [];
  for (const output of [...parent.outputs.filter((output) => below.has(workKey(output.source))), ...child.outputs]) {
    const key = `${output.kind}:${output.repository}:${output.number}`;
    if (seen.has(key)) continue;
    seen.add(key);
    rows.push(output);
  }
  return rows;
}

/** The PR mark's state for a recorded output: the record keeps merged, closed, CI failed, and otherwise open for review. */
export function outputPrState(state: string): PrState {
  return state === "merged" ? "merged" : state === "closed" ? "closed" : state === "ci_failed" ? "failed" : "pending";
}

export type TaskState = ReturnType<typeof taskState>;
/** One requested task, its derived verdict and the original request that asked for it. */
export type ChecklistTask = { task: WorkTask; state: TaskState; request: WorkRequest | null };
export type Checklist = {
  tasks: ChecklistTask[];
  done: number;
  total: number;
  latest: WorkRequest | null;
};

/** Requested work, first-asked first; task state uses only this session's outputs. */
export function checklist(record: SessionRecord): Checklist {
  const byAt = <T extends { at_unix_ms: number }>(rows: readonly T[]) => [...rows].sort((a, b) => a.at_unix_ms - b.at_unix_ms);
  const tasks = byAt(record.tasks);
  const outputs = ownOutputs(record);
  const taskIds = new Set(tasks.map((task) => task.id));
  const requests = byAt(record.requests);
  const requestOf = (task: WorkTask) => requests.find((request) => request.task_ids.some((id) => taskIds.has(id) && taskGroup(id) === taskGroup(task.id))) ?? null;
  const latest = record.requests.reduce<WorkRequest | null>((latest, request) => (!latest || request.offset > latest.offset ? request : latest), null);
  return {
    tasks: tasks.map((task) => ({ task, state: taskState(task, outputs), request: requestOf(task) })),
    done: tasks.filter((task) => task.done).length,
    total: tasks.length,
    latest: record.state === "working" && latest && !latest.judged ? latest : null,
  };
}

/** Where a session works: the project and checkout its pane is in, else the registered checkout holding its folder. */
export type SessionPlace = { project: Workspace; checkout: Checkout | null; agent: AgentRow | null };

/**
 * The project a session works in on its own device (PRD overview-device-tasks
 * B17, B44): its live pane's checkout when it has one, else the deepest
 * checkout or project path that holds its working folder. Null when no
 * registered project of that device holds it.
 */
export function sessionPlace(projects: readonly BoardProject[], session: WorkSession, cwd: string | null, rest: SnapshotRest | null): SessionPlace | null {
  const own = projects.filter(({ workspace }) => workspace.device_id === session.device_id);
  for (const { workspace, agents } of own) {
    const agent = agents.find((row) => row.agent_kind === session.agent && row.session_id === session.session_id && (row.device_id ?? localDeviceId(rest)) === session.device_id);
    const member = agent ? workspace.agent_scope.members.find((row) => row.pane_id === agent.pane_id) : undefined;
    if (!agent || !member) continue;
    return { project: workspace, checkout: workspace.checkouts.find((row) => row.id === member.checkout_id) ?? null, agent };
  }
  if (!cwd) return null;
  const holds = (path: string) => cwd === path || cwd.startsWith(path.endsWith("/") ? path : `${path}/`);
  let best: SessionPlace | null = null;
  let depth = -1;
  for (const { workspace } of own) {
    for (const checkout of workspace.checkouts) if (holds(checkout.path) && checkout.path.length > depth) { best = { project: workspace, checkout, agent: null }; depth = checkout.path.length; }
    if (holds(workspace.path) && workspace.path.length > depth) { best = { project: workspace, checkout: null, agent: null }; depth = workspace.path.length; }
  }
  return best;
}
