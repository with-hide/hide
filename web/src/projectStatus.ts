// The Project tool's rules (PRD project-panel-results D-03..D-16, B1-B37):
// which project it is about, how its 흘린 것 and tasks are cut and named,
// and how its cost reads. The core derives every row and number
// (`agent_state/tally/project_status.rs`, `usage_ledger/project_cost.rs`);
// these functions only shape what it sent, so they are testable without a
// page.

import type { ProjectStray } from "./agentScope";
import type { NodeUsage, ProjectCost, TaskCost } from "./generated/hided-ws";
import { formatUnit } from "./i18n/format";
import type { InterfaceLanguage } from "./i18n/locale";
import { openMissedWork } from "./sessionWork";
import { catalogWorkspaces, frontCheckout, type AgentRow, type LinkSummaries, type SnapshotRest, type Workspace } from "./snapshot";

/** The 흘린 것 rows shown before `+N 더` (B8). */
export const STRAYS_SHOWN = 5;

/** The tasks the panel lists as the costliest (B25). */
export const COSTLY_SHOWN = 3;

/** The Project of the checkout in front, or null at Home or in a device's Home folder. */
export function frontProject(rest: SnapshotRest | null): Workspace | null {
  const front = frontCheckout(rest);
  if (!front) return null;
  const project = catalogWorkspaces(rest).find((row) => row.id === front.workspace_id && row.checkouts.some((checkout) => checkout.id === front.id));
  return project && !project.is_home ? project : null;
}

/** The 흘린 것 rows drawn (B8): five until `open`, and how many the core sent past them or left past its cap. */
export function strayView(strays: readonly ProjectStray[], total: number, open: boolean): { shown: ProjectStray[]; more: number } {
  const shown = open ? [...strays] : strays.slice(0, STRAYS_SHOWN);
  return { shown, more: total - shown.length };
}

/** What a 흘린 것 row says is left (B6), in its order: changes, then commits, then the failed pull request. */
export type StrayPart = { kind: "changes"; count: number } | { kind: "unpushed"; count: number } | { kind: "ci"; number: number };

export function strayParts(stray: ProjectStray): StrayPart[] {
  const parts: StrayPart[] = [];
  if (stray.changed_files > 0) parts.push({ kind: "changes", count: stray.changed_files });
  if (stray.unpushed > 0) parts.push({ kind: "unpushed", count: stray.unpushed });
  if (stray.failed_pr != null) parts.push({ kind: "ci", number: stray.failed_pr });
  return parts;
}

/** The project's worktrees the disk line offers to clean (B18): the ones whose mark says they can go, and their measured size. */
export function removableDisk(project: Workspace): { count: number; bytes: number } {
  const rows = project.checkouts.filter((checkout) => checkout.agent_scope.git_mark === "removable");
  return { count: rows.length, bytes: rows.reduce((sum, checkout) => sum + (checkout.worktree?.disk?.total_bytes ?? 0), 0) };
}

/** The open missed work of a project (B4), none while Hide AI's summaries are off. */
export function missedCount(links: LinkSummaries | null, projectId: string, enabled: boolean): number {
  return enabled ? openMissedWork(links?.projects[projectId]?.work ?? []).length : 0;
}

/**
 * A project's cost as the core's usage frame carries it (B23): `pending`
 * before the core's first build, `failed` when no build has succeeded, or
 * `ready` with the cost (null when no call ran there in seven days) and
 * whether the latest build failed, the last one kept.
 */
export type CostRead = { state: "pending" } | { state: "failed" } | { state: "ready"; cost: ProjectCost | null; failed: boolean };

export function projectCost(node: Pick<NodeUsage, "projects" | "projects_failed"> | undefined, projectId: string): CostRead {
  if (node?.projects === undefined) return node?.projects_failed ? { state: "failed" } : { state: "pending" };
  return { state: "ready", cost: node.projects.find((row) => row.workspace_id === projectId) ?? null, failed: node.projects_failed === true };
}

/** Millionths of a dollar as money, to the cent. */
export function usd(micro: number, language: InterfaceLanguage): string {
  return new Intl.NumberFormat(language, { style: "currency", currency: "USD", currencyDisplay: "narrowSymbol", maximumFractionDigits: 2 }).format(micro / 1_000_000);
}

/** An amount the core said is a lower bound reads `≥`; an exact conversion reads `≈` (`UsageCost`). */
export function costPrefix(cost: Pick<ProjectCost, "partial" | "waiting" | "limited">): "≥" | "≈" {
  return cost.partial || cost.waiting || cost.limited ? "≥" : "≈";
}

/** A bar's height in the day chart: its share of the largest day, 0 for none. */
export function dayHeights(days: readonly { micro_usd: number }[]): number[] {
  const largest = Math.max(0, ...days.map((day) => day.micro_usd));
  return days.map((day) => (largest === 0 ? 0 : day.micro_usd / largest));
}

/** Tokens in a short form (`6.2M`). */
export function tokenWords(language: InterfaceLanguage, tokens: number): string {
  return new Intl.NumberFormat(language, { notation: "compact", maximumFractionDigits: 1 }).format(tokens);
}

/** A span of time in minutes below an hour, else hours to one decimal, else days past two (`3.4h`). */
export function durationWords(language: InterfaceLanguage, ms: number): string {
  const minutes = Math.max(0, Math.round(ms / 60_000));
  if (minutes < 60) return formatUnit(language, minutes, "minute");
  const hours = ms / 3_600_000;
  if (hours < 48) return formatUnit(language, Math.round(hours * 10) / 10, "hour");
  return formatUnit(language, Math.round(hours / 24), "day");
}

/** The 작업 비용 table's filters (B27). */
export const TASK_FILTERS = ["all", "merged", "going", "mission", "none"] as const;
export type TaskFilter = (typeof TASK_FILTERS)[number];

/**
 * Whether a task stands under `filter`: 머지됨 is a merged pull request, 진행
 * 중 work still on its way (an agent working, an open pull request, changes
 * left), 미션 a mission, 결과 없음 a task that left nothing (B30).
 */
export function taskIn(task: TaskCost, filter: TaskFilter): boolean {
  const result = task.result;
  switch (filter) {
    case "all": return true;
    case "merged": return result.kind === "pr" && result.state === "merged";
    case "going": return result.kind === "working" || result.kind === "changes" || (result.kind === "pr" && result.state === "open");
    case "mission": return task.kind === "mission";
    case "none": return result.kind === "none";
  }
}

/** Each filter's count over the tasks the core sent. */
export function filterCounts(tasks: readonly TaskCost[]): Record<TaskFilter, number> {
  return Object.fromEntries(TASK_FILTERS.map((filter) => [filter, tasks.filter((task) => taskIn(task, filter)).length])) as Record<TaskFilter, number>;
}

/**
 * What a task is called (B25, B27): a mission's title; else its root
 * session's name while the agent is listed, else the record's title; null
 * when nothing names it, which the caller words as the agent's session.
 */
export function taskName(task: Pick<TaskCost, "kind" | "mission" | "root">, agents: readonly Pick<AgentRow, "session_id" | "identity_label">[], links: LinkSummaries | null, projectId: string): string | null {
  if (task.kind === "mission" && task.mission) return task.mission.title;
  return sessionName(task.root.session_id, agents, links, projectId);
}

/** A session's name while its agent is listed, else its record's title. */
export function sessionName(sessionId: string, agents: readonly Pick<AgentRow, "session_id" | "identity_label">[], links: LinkSummaries | null, projectId: string): string | null {
  const listed = agents.find((row) => row.session_id === sessionId)?.identity_label;
  if (listed) return listed;
  const recorded = links?.projects[projectId]?.work?.find((work) => work.session.session_id === sessionId)?.title;
  return recorded || null;
}
