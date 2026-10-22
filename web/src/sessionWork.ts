import type { BoardProject } from "./projectBoard";
import { localDeviceId, type AgentRow, type Checkout, type Workspace, type MissedWork, type PrState, type SessionRecord, type SessionWorkSummary, type SnapshotRest, type WorkOutput, type WorkRequest, type WorkSession } from "./snapshot";

export const workKey = (session: WorkSession) => `${session.device_id}\0${session.agent}\0${session.session_id}`;
export function sessionOfAgent(agent: AgentRow, rest: SnapshotRest | null): WorkSession | null {
  return sessionOfAgentOn(agent, localDeviceId(rest));
}
/** The same, given this machine's id, for a caller that should not read the whole snapshot. */
export function sessionOfAgentOn(agent: AgentRow, localId: string): WorkSession | null {
  return agent.session_id ? { device_id: agent.device_id ?? localId, agent: agent.agent_kind, session_id: agent.session_id } : null;
}
export function recordTitle(record: Pick<SessionRecord, "title" | "manual_title" | "native_title" | "fallback_title">, summaries: boolean): string {
  return record.manual_title ?? (summaries ? record.title : record.native_title ?? record.fallback_title);
}
export function recordTree(record: SessionRecord): SessionRecord[] {
  return [record, ...record.children.flatMap(recordTree)];
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

/** The PR mark's state for a recorded output: the record keeps merged, closed, CI failed, and otherwise open for review. */
export function outputPrState(state: string): PrState {
  return state === "merged" ? "merged" : state === "closed" ? "closed" : state === "ci_failed" ? "failed" : "pending";
}

/** How many characters of a request with no summary show before `…` (D-06, B5). */
export const PREVIEW_CHARS = 120;

/** A request's words on one line: its summary, else its first characters with line breaks as spaces. */
export function requestPreview(request: Pick<WorkRequest, "summary" | "text">): { text: string; summarized: boolean; cut: boolean } {
  if (request.summary) return { text: request.summary, summarized: true, cut: false };
  const flat = request.text.replace(/\s+/g, " ").trim();
  const chars = [...flat];
  return chars.length > PREVIEW_CHARS ? { text: `${chars.slice(0, PREVIEW_CHARS).join("")}…`, summarized: false, cut: true } : { text: flat, summarized: false, cut: false };
}

/** A pull request nothing more happens to: merged, or closed unmerged (B19). */
/** Whether a record line is the message at `offset`, or a repeat merged into it (B23). */
export function holdsRequest(request: Pick<WorkRequest, "offset" | "repeats">, offset: number): boolean {
  return request.offset === offset || (request.repeats ?? []).includes(offset);
}

export const settledOutput = (output: Pick<WorkOutput, "state">) => output.state === "merged" || output.state === "closed";

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
